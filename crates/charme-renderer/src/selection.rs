use std::collections::HashMap;

use bevy::{
    math::Ray3d,
    prelude::{Resource, Vec3},
};
use charme_core::MaterialSlotId;

use crate::{pmx_import::PreparedPmxScene, scene::PmxMaterialSlot, source::PmxSourceIdentity};

/// CPU geometry retained for viewport picking and selected-primitive outlines.
#[derive(Resource, Default)]
pub(crate) struct SelectionGeometry {
    pub(crate) scene_source: Option<PmxSourceIdentity>,
    pub(crate) primitives: Vec<PrimitiveSelectionGeometry>,
    pub(crate) selected_slot: Option<MaterialSlotId>,
    pub(crate) selected_primitives: Vec<usize>,
}

pub(crate) struct PrimitiveSelectionGeometry {
    pub(crate) primitive_index: usize,
    pub(crate) slot_id: MaterialSlotId,
    pub(crate) components: Vec<PrimitiveComponentSelectionGeometry>,
}

pub(crate) struct PrimitiveComponentSelectionGeometry {
    pub(crate) faces: Vec<SelectionFace>,
    pub(crate) edges: Vec<SelectionEdge>,
}

#[derive(Clone, Copy)]
pub(crate) struct SelectionFace {
    pub(crate) vertices: [Vec3; 3],
    pub(crate) normal: Vec3,
}

pub(crate) struct SelectionEdge {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    /// Blender-style edge sharpness (`wd`): 0 for boundary/non-manifold and
    /// sharp edges, approaching 1 for flat edges. See `edge_sharpness`.
    pub(crate) sharpness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickedPrimitive {
    pub(crate) primitive_index: usize,
    pub(crate) slot_id: MaterialSlotId,
    pub(crate) distance: f32,
}

impl SelectionGeometry {
    pub(crate) fn from_prepared_with_progress(
        prepared: &PreparedPmxScene,
        mut report: impl FnMut(usize, usize),
    ) -> Self {
        let center = (prepared.bounds_min + prepared.bounds_max) * 0.5;
        let translation = Vec3::new(-center.x, -prepared.bounds_min.y, -center.z);
        let positions = &prepared.model.geometry().positions;
        let slots = prepared.info.material_slots();
        let total = prepared.model.primitives().len();
        let mut primitives = Vec::with_capacity(total);
        report(0, total);

        for (primitive_index, primitive) in prepared.model.primitives().iter().enumerate() {
            let Some(slot_id) = slots.get(primitive.material_index).map(PmxMaterialSlot::id) else {
                report(primitive_index + 1, total);
                continue;
            };
            let Some(split) = prepared
                .primitive_splits
                .get(primitive_index)
                .and_then(Option::as_ref)
            else {
                report(primitive_index + 1, total);
                continue;
            };
            let components = split
                .components
                .iter()
                .filter_map(|component| {
                    let faces = component
                        .indices
                        .chunks_exact(3)
                        .filter_map(|triangle| {
                            let first = positions.get(triangle[0] as usize)?;
                            let second = positions.get(triangle[1] as usize)?;
                            let third = positions.get(triangle[2] as usize)?;
                            Some(selection_face(
                                Vec3::from(*first) + translation,
                                Vec3::from(*second) + translation,
                                Vec3::from(*third) + translation,
                            ))
                        })
                        .collect::<Vec<_>>();
                    if faces.is_empty() {
                        return None;
                    }
                    let edges = selection_edges(&faces);
                    Some(PrimitiveComponentSelectionGeometry { faces, edges })
                })
                .collect::<Vec<_>>();
            if !components.is_empty() {
                primitives.push(PrimitiveSelectionGeometry {
                    primitive_index,
                    slot_id,
                    components,
                });
            }
            report(primitive_index + 1, total);
        }

        Self {
            scene_source: Some(prepared.info.source_identity().clone()),
            primitives,
            selected_slot: None,
            selected_primitives: Vec::new(),
        }
    }

    pub(crate) fn set_selected_slot(&mut self, slot_id: Option<MaterialSlotId>) -> bool {
        let selected_slot = slot_id.filter(|slot_id| {
            self.primitives
                .iter()
                .any(|primitive| primitive.slot_id == *slot_id)
        });
        let changed = self.selected_slot != selected_slot || !self.selected_primitives.is_empty();
        self.selected_slot = selected_slot;
        self.selected_primitives.clear();
        changed
    }

    pub(crate) fn set_selected_primitives(&mut self, primitive_indices: Vec<usize>) -> bool {
        let mut selected_primitives = primitive_indices
            .into_iter()
            .filter(|primitive_index| {
                self.primitives
                    .iter()
                    .any(|primitive| primitive.primitive_index == *primitive_index)
            })
            .collect::<Vec<_>>();
        selected_primitives.sort_unstable();
        selected_primitives.dedup();
        let changed =
            self.selected_primitives != selected_primitives || self.selected_slot.is_some();
        self.selected_primitives = selected_primitives;
        self.selected_slot = None;
        changed
    }

    pub(crate) fn selected_slot(&self) -> Option<MaterialSlotId> {
        self.selected_slot
    }

    pub(crate) fn selected_primitives(&self) -> &[usize] {
        &self.selected_primitives
    }

    pub(crate) fn pick(&self, ray: Ray3d) -> Option<PickedPrimitive> {
        let direction = *ray.direction;
        let mut closest = None;
        for primitive in &self.primitives {
            for component in &primitive.components {
                for face in &component.faces {
                    let Some(distance) =
                        ray_triangle_intersection(ray.origin, direction, face.vertices)
                    else {
                        continue;
                    };
                    if closest
                        .as_ref()
                        .is_none_or(|hit: &PickedPrimitive| distance < hit.distance)
                    {
                        closest = Some(PickedPrimitive {
                            primitive_index: primitive.primitive_index,
                            slot_id: primitive.slot_id,
                            distance,
                        });
                    }
                }
            }
        }
        closest
    }
}

pub(crate) fn selection_face(first: Vec3, second: Vec3, third: Vec3) -> SelectionFace {
    let raw_normal = (second - first).cross(third - first);
    let normal = if raw_normal.length_squared() > f32::EPSILON {
        raw_normal.normalize()
    } else {
        Vec3::ZERO
    };
    SelectionFace {
        vertices: [first, second, third],
        normal,
    }
}

/// Mirrors Blender's `edge_factor_calc` (extract_mesh_vbo_edge_fac.cc): the
/// cosine of the dihedral angle rescaled so that edges sharper than the
/// default threshold collapse to 0 and flat edges approach 1.
fn edge_sharpness(first: Vec3, second: Vec3) -> f32 {
    let cosine = first.dot(second);
    let factor = (200.0 * (cosine - 1.0) + 1.0).clamp(0.0, 1.0);
    factor * (254.0 / 255.0)
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PositionKey([u32; 3]);

impl From<Vec3> for PositionKey {
    fn from(value: Vec3) -> Self {
        Self([value.x.to_bits(), value.y.to_bits(), value.z.to_bits()])
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct EdgeKey(PositionKey, PositionKey);

struct EdgeAccumulator {
    start: Vec3,
    end: Vec3,
    face_normals: Vec<Vec3>,
}

pub(crate) fn selection_edges(faces: &[SelectionFace]) -> Vec<SelectionEdge> {
    let mut edges = HashMap::<EdgeKey, EdgeAccumulator>::new();
    for face in faces {
        for (start, end) in [
            (face.vertices[0], face.vertices[1]),
            (face.vertices[1], face.vertices[2]),
            (face.vertices[2], face.vertices[0]),
        ] {
            let start_key = PositionKey::from(start);
            let end_key = PositionKey::from(end);
            let (key, start, end) = if start_key <= end_key {
                (EdgeKey(start_key, end_key), start, end)
            } else {
                (EdgeKey(end_key, start_key), end, start)
            };
            let edge = edges.entry(key).or_insert_with(|| EdgeAccumulator {
                start,
                end,
                face_normals: Vec::new(),
            });
            edge.face_normals.push(face.normal);
        }
    }

    edges
        .into_values()
        .map(|edge| {
            // Boundary and non-manifold edges are always visible, matching
            // Blender's reserved values.
            let sharpness = match edge.face_normals.as_slice() {
                [first, second] if *first != Vec3::ZERO && *second != Vec3::ZERO => {
                    edge_sharpness(*first, *second)
                }
                _ => 0.0,
            };
            SelectionEdge {
                start: edge.start,
                end: edge.end,
                sharpness,
            }
        })
        .collect()
}

fn ray_triangle_intersection(origin: Vec3, direction: Vec3, vertices: [Vec3; 3]) -> Option<f32> {
    const EPSILON: f32 = 1e-6;
    let edge_one = vertices[1] - vertices[0];
    let edge_two = vertices[2] - vertices[0];
    let perpendicular = direction.cross(edge_two);
    let determinant = edge_one.dot(perpendicular);
    if determinant.abs() < EPSILON {
        return None;
    }

    let inverse_determinant = determinant.recip();
    let origin_offset = origin - vertices[0];
    let u = inverse_determinant * origin_offset.dot(perpendicular);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let cross = origin_offset.cross(edge_one);
    let v = inverse_determinant * direction.dot(cross);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let distance = inverse_determinant * edge_two.dot(cross);
    (distance > EPSILON).then_some(distance)
}
