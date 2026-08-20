use charme_core::{CharacterSource, MaterialSlotId};
use charme_renderer::{PmxLoadProgress, PmxSourceIdentity, ViewportSelectionAction};

/// Semantic level used when interpreting hierarchy and viewport selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SelectionLevel {
    /// Select imported material slots.
    #[default]
    MaterialSlot,
    /// Select one or more source primitives.
    Primitive,
}

/// Platform-independent editor selection state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectionState {
    level: SelectionLevel,
    material_slot: Option<MaterialSlotId>,
    primitives: Vec<usize>,
}

impl SelectionState {
    /// Returns the active selection level.
    pub const fn level(&self) -> SelectionLevel {
        self.level
    }

    /// Changes the selection level and clears targets from the previous level.
    pub fn set_level(&mut self, level: SelectionLevel) -> bool {
        if self.level == level {
            return false;
        }
        self.level = level;
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
pub struct PmxImportTracker {
    next_request_id: u64,
    pending: Option<PendingPmxImport>,
}

impl PmxImportTracker {
    /// Starts a request and returns its monotonically increasing identifier.
    pub fn begin(&mut self, source: PmxSourceIdentity, character: Option<CharacterSource>) -> u64 {
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let request_id = self.next_request_id;
        self.pending = Some(PendingPmxImport {
            request_id,
            source,
            character,
        });
        request_id
    }

    /// Invalidates any pending request.
    pub fn invalidate(&mut self) {
        self.pending = None;
        self.next_request_id = self.next_request_id.wrapping_add(1);
    }

    /// Returns true when progress belongs to the active request.
    pub fn accepts(&self, progress: &PmxLoadProgress) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.request_id == progress.request_id()
                && sources_match(&pending.source, progress.source_identity())
        })
    }

    /// Completes and returns the active request when identity and ID match.
    pub fn complete(
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

/// Application-owned transient workspace state shared by native frontends.
#[derive(Debug, Default)]
pub struct WorkspaceState {
    selection: SelectionState,
    pmx_import: PmxImportTracker,
    loaded_scene: Option<(u64, PmxSourceIdentity)>,
}

impl WorkspaceState {
    /// Returns current selection state.
    pub const fn selection(&self) -> &SelectionState {
        &self.selection
    }

    /// Mutably borrows current selection state.
    pub fn selection_mut(&mut self) -> &mut SelectionState {
        &mut self.selection
    }

    /// Returns the PMX import tracker.
    pub const fn pmx_import(&self) -> &PmxImportTracker {
        &self.pmx_import
    }

    /// Mutably borrows the PMX import tracker.
    pub fn pmx_import_mut(&mut self) -> &mut PmxImportTracker {
        &mut self.pmx_import
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
        let mut state = SelectionState::default();
        state.set_level(SelectionLevel::Primitive);
        state.apply_primitive(ViewportSelectionAction::Toggle, Some(3));
        state.apply_primitive(ViewportSelectionAction::Toggle, Some(1));
        assert_eq!(state.primitives(), &[1, 3]);

        state.apply_primitive(ViewportSelectionAction::Remove, Some(3));
        assert_eq!(state.primitives(), &[1]);
    }

    #[test]
    fn import_tracker_accepts_only_the_latest_request() {
        let mut tracker = PmxImportTracker::default();
        let old_source = PmxSourceIdentity::file("old.pmx");
        let old = tracker.begin(old_source.clone(), None);
        let new_source = PmxSourceIdentity::file("new.pmx");
        let new = tracker.begin(new_source.clone(), None);

        assert!(tracker.complete(old, &old_source).is_none());
        assert!(tracker.complete(new, &new_source).is_some());
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
