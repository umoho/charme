use std::path::Path;

use charme_core::MaterialSlotId;

use crate::source::PmxSourceIdentity;

#[cfg(test)]
use crate::{
    pmx_import::{build_primitive_splits, primitive_component_infos},
    selection::{
        PrimitiveComponentSelectionGeometry, PrimitiveSelectionGeometry, SelectionGeometry,
        selection_edges, selection_face,
    },
};
#[cfg(test)]
use bevy::prelude::Vec3;
#[cfg(test)]
use bevy_pmx::Pmx;

/// A PMX material slot exposed to the editor UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxMaterialSlot {
    pub(crate) id: MaterialSlotId,
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) english_name: String,
    pub(crate) diffuse_texture: Option<String>,
    pub(crate) sphere_texture: Option<String>,
    pub(crate) toon_texture: Option<String>,
}

impl PmxMaterialSlot {
    /// Returns the stable identifier assigned to this imported slot.
    pub const fn id(&self) -> MaterialSlotId {
        self.id
    }

    /// Returns the material's zero-based index in the PMX document.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns the material's primary PMX name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional English material name stored in the PMX document.
    pub fn english_name(&self) -> &str {
        &self.english_name
    }

    /// Returns the original diffuse texture path from the PMX document.
    pub fn diffuse_texture(&self) -> Option<&str> {
        self.diffuse_texture.as_deref()
    }

    /// Returns the original sphere texture path from the PMX document.
    pub fn sphere_texture(&self) -> Option<&str> {
        self.sphere_texture.as_deref()
    }

    /// Returns the original toon texture path, or a shared-toon identifier.
    pub fn toon_texture(&self) -> Option<&str> {
        self.toon_texture.as_deref()
    }
}

/// UI-facing summary of one connected component within a PMX primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxPrimitiveComponentInfo {
    pub(crate) index: usize,
    pub(crate) triangle_count: usize,
    pub(crate) index_count: usize,
    pub(crate) vertex_count: usize,
}

impl PmxPrimitiveComponentInfo {
    /// Returns the component's zero-based index within its primitive.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns the number of triangles in the component.
    pub const fn triangle_count(&self) -> usize {
        self.triangle_count
    }

    /// Returns the number of indices in the component.
    pub const fn index_count(&self) -> usize {
        self.index_count
    }

    /// Returns the number of distinct source vertices referenced by the component.
    pub const fn vertex_count(&self) -> usize {
        self.vertex_count
    }
}

/// UI-facing summary of one indexed PMX primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxPrimitiveInfo {
    pub(crate) index: usize,
    pub(crate) index_count: usize,
    pub(crate) material_slot_id: Option<MaterialSlotId>,
    pub(crate) components: Vec<PmxPrimitiveComponentInfo>,
}

impl PmxPrimitiveInfo {
    /// Returns the primitive's zero-based index in the PMX model.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns the number of indices occupied by the primitive.
    pub const fn index_count(&self) -> usize {
        self.index_count
    }

    /// Returns the material slot assigned to the primitive, when valid.
    pub const fn material_slot_id(&self) -> Option<MaterialSlotId> {
        self.material_slot_id
    }

    /// Returns connected components in source triangle order.
    pub fn components(&self) -> &[PmxPrimitiveComponentInfo] {
        &self.components
    }
}

/// UI-facing summary of a loaded PMX scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxSceneInfo {
    pub(crate) source: PmxSourceIdentity,
    pub(crate) name: String,
    pub(crate) vertex_count: usize,
    pub(crate) index_count: usize,
    pub(crate) material_slots: Vec<PmxMaterialSlot>,
    pub(crate) primitives: Vec<PmxPrimitiveInfo>,
    pub(crate) warnings: Vec<String>,
}

impl PmxSceneInfo {
    /// Returns the complete runtime identity of the loaded PMX source.
    pub fn source_identity(&self) -> &PmxSourceIdentity {
        &self.source
    }

    /// Returns the source PMX path or containing ZIP archive path.
    pub fn path(&self) -> &Path {
        self.source.path()
    }

    /// Returns the selected PMX entry inside the source ZIP archive, if any.
    pub fn archive_entry(&self) -> Option<&str> {
        self.source.archive_entry()
    }

    /// Returns the model name, falling back to the selected PMX entry or source file name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the number of vertices in the imported model.
    pub const fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    /// Returns the number of triangle indices in the imported model.
    pub const fn index_count(&self) -> usize {
        self.index_count
    }

    /// Returns material slots in PMX document order.
    pub fn material_slots(&self) -> &[PmxMaterialSlot] {
        &self.material_slots
    }

    /// Returns indexed primitives in PMX document order.
    pub fn primitives(&self) -> &[PmxPrimitiveInfo] {
        &self.primitives
    }

    /// Returns recoverable import warnings, such as missing textures.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Dir3, Ray3d};
    use bevy_pmx::{PmxMeshGeometry, PmxPrimitive};

    #[test]
    fn primitive_components_are_summarized_in_source_order() {
        let model = Pmx::new(
            None,
            PmxMeshGeometry {
                positions: vec![[0.0, 0.0, 0.0]; 6],
                normals: vec![[0.0, 0.0, 1.0]; 6],
                uvs: vec![[0.0, 0.0]; 6],
                indices: vec![0, 1, 2, 3, 4, 5],
            },
            vec![PmxPrimitive {
                material_index: 0,
                index_start: 0,
                index_count: 6,
            }],
        );
        let splits = build_primitive_splits(&model);
        let split = splits[0]
            .as_ref()
            .expect("valid primitive should have topology data");

        assert_eq!(split.triangle_components, [0, 1]);
        assert_eq!(
            primitive_component_infos(split)
                .iter()
                .map(PmxPrimitiveComponentInfo::triangle_count)
                .collect::<Vec<_>>(),
            [1, 1]
        );
    }

    #[test]
    fn shared_triangle_edges_are_deduplicated() {
        let faces = vec![
            selection_face(
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
            ),
            selection_face(
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
            ),
        ];
        let edges = selection_edges(&faces);

        // Two triangles sharing a diagonal contribute five unique edges.
        assert_eq!(edges.len(), 5);
    }

    #[test]
    fn picking_returns_the_nearest_primitive() {
        let slot = MaterialSlotId::new();
        let faces = vec![selection_face(
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )];
        let geometry = SelectionGeometry {
            scene_source: Some(PmxSourceIdentity::file("model.pmx")),
            primitives: vec![PrimitiveSelectionGeometry {
                primitive_index: 3,
                slot_id: slot,
                components: vec![PrimitiveComponentSelectionGeometry {
                    edges: selection_edges(&faces),
                    faces,
                }],
            }],
            selected_slot: None,
            selected_primitives: Vec::new(),
        };
        let ray = Ray3d::new(
            Vec3::new(0.0, 0.0, 2.0),
            Dir3::new(Vec3::NEG_Z).expect("negative Z is a valid direction"),
        );

        let picked = geometry.pick(ray).expect("ray should hit the triangle");
        assert_eq!(picked.primitive_index, 3);
        assert_eq!(picked.slot_id, slot);
        assert!((picked.distance - 2.0).abs() < 1e-6);
    }

    #[test]
    fn selected_slot_is_limited_to_loaded_primitives() {
        let slot = MaterialSlotId::new();
        let geometry = SelectionGeometry {
            scene_source: None,
            primitives: vec![PrimitiveSelectionGeometry {
                primitive_index: 0,
                slot_id: slot,
                components: Vec::new(),
            }],
            selected_slot: None,
            selected_primitives: Vec::new(),
        };
        let mut geometry = geometry;

        assert!(geometry.set_selected_slot(Some(slot)));
        assert_eq!(geometry.selected_slot(), Some(slot));
        assert!(geometry.set_selected_slot(None));
        assert_eq!(geometry.selected_slot(), None);
    }

    #[test]
    fn primitive_selection_is_limited_and_exclusive() {
        let slot = MaterialSlotId::new();
        let mut geometry = SelectionGeometry {
            scene_source: None,
            primitives: vec![PrimitiveSelectionGeometry {
                primitive_index: 3,
                slot_id: slot,
                components: Vec::new(),
            }],
            selected_slot: None,
            selected_primitives: Vec::new(),
        };

        assert!(!geometry.set_selected_primitives(vec![9]));
        assert!(geometry.selected_primitives().is_empty());
        assert!(geometry.set_selected_slot(Some(slot)));
        assert!(geometry.set_selected_primitives(vec![3, 3]));
        assert_eq!(geometry.selected_primitives(), [3]);
        assert_eq!(geometry.selected_slot(), None);
        assert!(geometry.set_selected_primitives(Vec::new()));
        assert!(geometry.selected_primitives().is_empty());
    }
}
