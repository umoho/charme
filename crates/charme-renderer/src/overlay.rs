use std::collections::BTreeSet;

use bevy::prelude::{App, ChildOf, Entity, Name};

/// Renderer-owned transient overlays that never alter document or source scene data.
#[derive(Debug, Default)]
pub(crate) struct PreviewOverlays {
    entities: Vec<Entity>,
    connectivity_primitives: BTreeSet<usize>,
}

impl PreviewOverlays {
    /// Adds lightweight hierarchy markers for connected primitive components.
    pub(crate) fn show_connectivity(
        &mut self,
        app: &mut App,
        primitive_entities: &[Option<Entity>],
        component_counts: &[usize],
        primitive_indices: &[usize],
    ) -> bool {
        let mut requested = primitive_indices.to_vec();
        requested.sort_unstable();
        requested.dedup();

        let mut changed = false;
        for primitive_index in requested {
            if self.connectivity_primitives.contains(&primitive_index) {
                continue;
            }
            let Some(Some(original_entity)) = primitive_entities.get(primitive_index) else {
                continue;
            };
            let Some(&component_count) = component_counts.get(primitive_index) else {
                continue;
            };
            if component_count <= 1 {
                continue;
            }

            // Components retain the original mesh/material draw. Child markers
            // expose the split to hierarchy projections without multiplying GPU work.
            for component_index in 0..component_count {
                let entity = app
                    .world_mut()
                    .spawn((
                        Name::new(format!(
                            "PMX Primitive {primitive_index} Component {component_index}"
                        )),
                        ChildOf(*original_entity),
                    ))
                    .id();
                self.entities.push(entity);
            }
            self.connectivity_primitives.insert(primitive_index);
            changed = true;
        }
        changed
    }

    pub(crate) fn despawn(self, app: &mut App) {
        for entity in self.entities {
            let _ = app.world_mut().despawn(entity);
        }
    }
}
