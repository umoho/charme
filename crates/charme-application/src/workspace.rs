use charme_core::{CharacterSource, MaterialSlotId};
use charme_renderer::{PmxLoadProgress, PmxSourceIdentity, ViewportSelectionAction};

use crate::tool::ViewportToolId;

/// Platform-independent editor selection state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectionState {
    tool: ViewportToolId,
    material_slot: Option<MaterialSlotId>,
    primitives: Vec<usize>,
}

impl SelectionState {
    /// Returns the active viewport tool.
    pub const fn tool(&self) -> ViewportToolId {
        self.tool
    }

    /// Changes the active tool and clears targets from the previous tool.
    pub fn set_tool(&mut self, tool: ViewportToolId) -> bool {
        if self.tool == tool {
            return false;
        }
        self.tool = tool;
        self.clear();
        true
    }

    /// Returns the selected material slot.
    pub const fn material_slot(&self) -> Option<MaterialSlotId> {
        self.material_slot
    }

    /// Returns selected primitive indices in sorted order.
    pub fn primitives(&self) -> &[usize] {
        &self.primitives
    }

    /// Selects one material slot and clears primitive selection.
    pub fn select_material_slot(&mut self, slot: Option<MaterialSlotId>) -> bool {
        let changed = self.material_slot != slot || !self.primitives.is_empty();
        self.material_slot = slot;
        self.primitives.clear();
        changed
    }

    /// Replaces primitive selection with valid caller-provided indices.
    pub fn select_primitives(&mut self, mut primitives: Vec<usize>) -> bool {
        primitives.sort_unstable();
        primitives.dedup();
        let changed = self.primitives != primitives || self.material_slot.is_some();
        self.primitives = primitives;
        self.material_slot = None;
        changed
    }

    /// Applies one viewport operation to a material-slot hit.
    pub fn apply_material_slot(
        &mut self,
        operation: ViewportSelectionAction,
        hit: Option<MaterialSlotId>,
    ) -> bool {
        let next = match (operation, hit) {
            (ViewportSelectionAction::Replace, hit) => hit,
            (ViewportSelectionAction::Toggle, Some(hit)) if self.material_slot == Some(hit) => None,
            (ViewportSelectionAction::Toggle, Some(hit)) => Some(hit),
            (ViewportSelectionAction::Remove, Some(hit)) if self.material_slot == Some(hit) => None,
            (ViewportSelectionAction::Toggle | ViewportSelectionAction::Remove, _) => {
                self.material_slot
            }
        };
        self.select_material_slot(next)
    }

    /// Applies one viewport operation to a primitive hit.
    pub fn apply_primitive(
        &mut self,
        operation: ViewportSelectionAction,
        hit: Option<usize>,
    ) -> bool {
        let mut next = self.primitives.clone();
        match (operation, hit) {
            (ViewportSelectionAction::Replace, Some(hit)) => next = vec![hit],
            (ViewportSelectionAction::Replace, None) => next.clear(),
            (ViewportSelectionAction::Toggle, Some(hit)) => {
                if let Some(position) = next.iter().position(|candidate| *candidate == hit) {
                    next.remove(position);
                } else {
                    next.push(hit);
                }
            }
            (ViewportSelectionAction::Remove, Some(hit)) => {
                next.retain(|candidate| *candidate != hit);
            }
            (ViewportSelectionAction::Toggle | ViewportSelectionAction::Remove, None) => {}
        }
        self.select_primitives(next)
    }

    /// Clears all selected targets without changing the active level.
    pub fn clear(&mut self) -> bool {
        let changed = self.material_slot.take().is_some() || !self.primitives.is_empty();
        self.primitives.clear();
        changed
    }
}

/// Candidate document state retained while a PMX import is in flight.
#[derive(Clone, Debug)]
pub struct PendingPmxImport {
    request_id: u64,
    source: PmxSourceIdentity,
    character: Option<CharacterSource>,
}

impl PendingPmxImport {
    /// Returns the application-owned request identifier.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Returns the requested source.
    pub const fn source(&self) -> &PmxSourceIdentity {
        &self.source
    }

    /// Takes the character candidate that should be committed on success.
    pub fn take_character(&mut self) -> Option<CharacterSource> {
        self.character.take()
    }
}

/// Tracks the latest PMX import and rejects stale progress or completion events.
#[derive(Debug, Default)]
struct PmxImportTracker {
    next_request_id: u64,
    pending: Option<PendingPmxImport>,
}

impl PmxImportTracker {
    fn begin(&mut self, source: PmxSourceIdentity, character: Option<CharacterSource>) -> u64 {
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let request_id = self.next_request_id;
        self.pending = Some(PendingPmxImport {
            request_id,
            source,
            character,
        });
        request_id
    }

    fn invalidate(&mut self) {
        self.pending = None;
        self.next_request_id = self.next_request_id.wrapping_add(1);
    }

    fn accepts(&self, progress: &PmxLoadProgress) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.request_id == progress.request_id()
                && sources_match(&pending.source, progress.source_identity())
        })
    }

    fn complete(
        &mut self,
        request_id: u64,
        source: &PmxSourceIdentity,
    ) -> Option<PendingPmxImport> {
        let matches = self.pending.as_ref().is_some_and(|pending| {
            pending.request_id == request_id && sources_match(&pending.source, source)
        });
        matches.then(|| self.pending.take()).flatten()
    }
}

/// Semantic action applied to transient workspace state.
#[derive(Debug)]
pub enum WorkspaceAction {
    /// Reset transient state for a new project.
    Reset,
    /// Change the active hierarchy and viewport selection tool.
    SetViewportTool(ViewportToolId),
    /// Clear all selected targets.
    ClearSelection,
    /// Select one imported material slot.
    SelectMaterialSlot(Option<MaterialSlotId>),
    /// Replace selected primitive indices.
    SelectPrimitives(Vec<usize>),
    /// Apply one viewport operation to a material-slot hit.
    ApplyMaterialViewport {
        /// Selection operation supplied by the viewport.
        operation: ViewportSelectionAction,
        /// Picked slot, if any.
        hit: Option<MaterialSlotId>,
    },
    /// Apply one viewport operation to a primitive hit.
    ApplyPrimitiveViewport {
        /// Selection operation supplied by the viewport.
        operation: ViewportSelectionAction,
        /// Picked primitive, if any.
        hit: Option<usize>,
    },
    /// Start a PMX import operation.
    BeginPmxImport {
        /// Requested source identity.
        source: PmxSourceIdentity,
        /// Candidate document character committed only on success.
        character: Option<CharacterSource>,
    },
    /// Accept or reject PMX loading progress.
    PmxProgress(PmxLoadProgress),
    /// Complete a PMX import operation.
    CompletePmxImport {
        /// Renderer request identifier.
        request_id: u64,
        /// Completed source identity.
        source: PmxSourceIdentity,
    },
}

/// Side effect emitted after a workspace action.
#[derive(Debug)]
pub enum WorkspaceEffect {
    /// Selection state changed and its UI/renderer projections should refresh.
    SelectionChanged,
    /// A PMX request started and should be submitted to the renderer.
    PmxImportStarted {
        /// Application-owned request identifier.
        request_id: u64,
        /// Requested source identity.
        source: PmxSourceIdentity,
    },
    /// Progress belongs to the active PMX request.
    PmxProgressAccepted(PmxLoadProgress),
    /// A matching PMX request completed.
    PmxImportCompleted(PendingPmxImport),
    /// Transient project state was reset.
    Reset,
}

/// Application-owned transient workspace state shared by native frontends.
#[derive(Debug, Default)]
pub struct WorkspaceState {
    selection: SelectionState,
    pmx_import: PmxImportTracker,
    loaded_scene: Option<(u64, PmxSourceIdentity)>,
}

impl WorkspaceState {
    /// Applies one semantic workspace action and returns required adapter effects.
    pub fn dispatch(&mut self, action: WorkspaceAction) -> Vec<WorkspaceEffect> {
        match action {
            WorkspaceAction::Reset => {
                self.reset();
                vec![WorkspaceEffect::SelectionChanged, WorkspaceEffect::Reset]
            }
            WorkspaceAction::SetViewportTool(tool) => self
                .selection
                .set_tool(tool)
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::ClearSelection => self
                .selection
                .clear()
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::SelectMaterialSlot(slot) => self
                .selection
                .select_material_slot(slot)
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::SelectPrimitives(primitives) => self
                .selection
                .select_primitives(primitives)
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::ApplyMaterialViewport { operation, hit } => self
                .selection
                .apply_material_slot(operation, hit)
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::ApplyPrimitiveViewport { operation, hit } => self
                .selection
                .apply_primitive(operation, hit)
                .then_some(WorkspaceEffect::SelectionChanged)
                .into_iter()
                .collect(),
            WorkspaceAction::BeginPmxImport { source, character } => {
                let request_id = self.pmx_import.begin(source.clone(), character);
                vec![WorkspaceEffect::PmxImportStarted { request_id, source }]
            }
            WorkspaceAction::PmxProgress(progress) => {
                if self.pmx_import.accepts(&progress) {
                    vec![WorkspaceEffect::PmxProgressAccepted(progress)]
                } else {
                    Vec::new()
                }
            }
            WorkspaceAction::CompletePmxImport { request_id, source } => self
                .pmx_import
                .complete(request_id, &source)
                .map(WorkspaceEffect::PmxImportCompleted)
                .into_iter()
                .collect(),
        }
    }

    /// Returns current selection state.
    pub const fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Records the scene installed by the renderer.
    pub fn install_scene(&mut self, request_id: u64, source: PmxSourceIdentity) {
        self.loaded_scene = Some((request_id, source));
    }

    /// Returns true when an asynchronous result belongs to the installed scene.
    pub fn scene_matches(&self, request_id: u64, source: &PmxSourceIdentity) -> bool {
        self.loaded_scene
            .as_ref()
            .is_some_and(|(active_id, active)| *active_id == request_id && active == source)
    }

    /// Resets all transient state for a newly opened project.
    pub fn reset(&mut self) {
        self.selection = SelectionState::default();
        self.pmx_import.invalidate();
        self.loaded_scene = None;
    }
}

fn sources_match(expected: &PmxSourceIdentity, actual: &PmxSourceIdentity) -> bool {
    expected.path() == actual.path()
        && expected
            .archive_entry()
            .is_none_or(|entry| Some(entry) == actual.archive_entry())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_reducer_applies_toggle_and_remove() {
        let mut workspace = WorkspaceState::default();
        workspace.dispatch(WorkspaceAction::SetViewportTool(
            ViewportToolId::SelectPrimitive,
        ));
        workspace.dispatch(WorkspaceAction::ApplyPrimitiveViewport {
            operation: ViewportSelectionAction::Toggle,
            hit: Some(3),
        });
        workspace.dispatch(WorkspaceAction::ApplyPrimitiveViewport {
            operation: ViewportSelectionAction::Toggle,
            hit: Some(1),
        });
        assert_eq!(workspace.selection().primitives(), &[1, 3]);

        let effects = workspace.dispatch(WorkspaceAction::ApplyPrimitiveViewport {
            operation: ViewportSelectionAction::Remove,
            hit: Some(3),
        });
        assert!(matches!(
            effects.as_slice(),
            [WorkspaceEffect::SelectionChanged]
        ));
        assert_eq!(workspace.selection().primitives(), &[1]);
    }

    #[test]
    fn import_tracker_accepts_only_the_latest_request() {
        let mut workspace = WorkspaceState::default();
        let old_source = PmxSourceIdentity::file("old.pmx");
        let old = workspace
            .dispatch(WorkspaceAction::BeginPmxImport {
                source: old_source.clone(),
                character: None,
            })
            .into_iter()
            .find_map(|effect| match effect {
                WorkspaceEffect::PmxImportStarted { request_id, .. } => Some(request_id),
                _ => None,
            })
            .unwrap();
        let new_source = PmxSourceIdentity::file("new.pmx");
        let new = workspace
            .dispatch(WorkspaceAction::BeginPmxImport {
                source: new_source.clone(),
                character: None,
            })
            .into_iter()
            .find_map(|effect| match effect {
                WorkspaceEffect::PmxImportStarted { request_id, .. } => Some(request_id),
                _ => None,
            })
            .unwrap();

        assert!(
            workspace
                .dispatch(WorkspaceAction::CompletePmxImport {
                    request_id: old,
                    source: old_source,
                })
                .is_empty()
        );
        assert!(matches!(
            workspace
                .dispatch(WorkspaceAction::CompletePmxImport {
                    request_id: new,
                    source: new_source,
                })
                .as_slice(),
            [WorkspaceEffect::PmxImportCompleted(_)]
        ));
    }

    #[test]
    fn workspace_matches_results_to_the_installed_scene() {
        let source = PmxSourceIdentity::zip("model.zip", "A/model.pmx");
        let mut workspace = WorkspaceState::default();
        workspace.install_scene(7, source.clone());

        assert!(workspace.scene_matches(7, &source));
        assert!(!workspace.scene_matches(8, &source));
    }
}
