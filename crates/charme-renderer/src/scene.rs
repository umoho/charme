use std::{collections::HashMap, io, path::Path};

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    math::Ray3d,
    mesh::{Indices, PrimitiveTopology},
    prelude::{
        AlphaMode, App, Assets, Entity, Handle, Image, Mesh, Mesh3d, MeshMaterial3d, Name,
        Resource, Transform, Vec3,
    },
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_pmx::{
    Pmx, PmxImportContext, PmxMaterialRecord, PmxMeshGeometry, PmxResolvedPath, import_pmx,
    parse_pmx,
};
use charme_bevy::{CharmeMaterial, CharmeMaterialParams};
use charme_core::MaterialSlotId;
use charme_geometry::{PrimitiveRange, PrimitiveSplit, split_primitive};

use crate::{
    PmxLoadStage,
    source::{PmxInputSource, PmxSourceIdentity, ResolvedPmxLoadRequest},
};

/// A PMX material slot exposed to the editor UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxMaterialSlot {
    id: MaterialSlotId,
    index: usize,
    name: String,
    english_name: String,
    diffuse_texture: Option<String>,
    sphere_texture: Option<String>,
    toon_texture: Option<String>,
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
    index: usize,
    triangle_count: usize,
    index_count: usize,
    vertex_count: usize,
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
    index: usize,
    index_count: usize,
    material_slot_id: Option<MaterialSlotId>,
    components: Vec<PmxPrimitiveComponentInfo>,
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
    source: PmxSourceIdentity,
    name: String,
    vertex_count: usize,
    index_count: usize,
    material_slots: Vec<PmxMaterialSlot>,
    primitives: Vec<PmxPrimitiveInfo>,
    warnings: Vec<String>,
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

pub(crate) struct PreparedPmxScene {
    pub info: PmxSceneInfo,
    model: Pmx,
    primitive_splits: Vec<Option<PrimitiveSplit>>,
    textures: Vec<DecodedTexture>,
    bounds_min: Vec3,
    bounds_max: Vec3,
}

impl PreparedPmxScene {
    pub fn normalized_bounds(&self) -> (Vec3, Vec3) {
        let center = (self.bounds_min + self.bounds_max) * 0.5;
        let translation = Vec3::new(-center.x, -self.bounds_min.y, -center.z);
        (self.bounds_min + translation, self.bounds_max + translation)
    }
}

/// CPU geometry retained for viewport picking and selected-primitive outlines.
#[derive(Resource, Default)]
pub(crate) struct SelectionGeometry {
    pub(crate) scene_source: Option<PmxSourceIdentity>,
    pub(crate) primitives: Vec<PrimitiveSelectionGeometry>,
    selected_slot: Option<MaterialSlotId>,
    selected_primitives: Vec<usize>,
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
    pub(crate) center: Vec3,
}

pub(crate) struct SelectionEdge {
    pub(crate) start: Vec3,
    pub(crate) end: Vec3,
    pub(crate) faces: Vec<usize>,
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

fn selection_face(first: Vec3, second: Vec3, third: Vec3) -> SelectionFace {
    let raw_normal = (second - first).cross(third - first);
    let normal = if raw_normal.length_squared() > f32::EPSILON {
        raw_normal.normalize()
    } else {
        Vec3::ZERO
    };
    SelectionFace {
        vertices: [first, second, third],
        normal,
        center: (first + second + third) / 3.0,
    }
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
    faces: Vec<usize>,
}

fn selection_edges(faces: &[SelectionFace]) -> Vec<SelectionEdge> {
    let mut edges = HashMap::<EdgeKey, EdgeAccumulator>::new();
    for (face_index, face) in faces.iter().enumerate() {
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
                faces: Vec::new(),
            });
            if !edge.faces.contains(&face_index) {
                edge.faces.push(face_index);
            }
        }
    }

    edges
        .into_values()
        .map(|edge| SelectionEdge {
            start: edge.start,
            end: edge.end,
            faces: edge.faces,
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

struct DecodedTexture {
    image: Image,
    has_alpha: bool,
}

pub(crate) fn prepare_pmx_scene(
    request: &ResolvedPmxLoadRequest,
    mut report: impl FnMut(PmxLoadStage, Option<usize>, Option<usize>),
) -> Result<PreparedPmxScene, String> {
    let source = request.source.as_ref();
    report(PmxLoadStage::ReadingPmx, None, None);
    let bytes = source.read_pmx_bytes().map_err(|error| {
        format!(
            "failed to read PMX file {} (resolved to {}): {error}",
            source.identity().path().display(),
            source.pmx_location()
        )
    })?;
    report(PmxLoadStage::ReadingPmx, Some(1), Some(1));

    // `bevy_pmx` does not expose parser callbacks. Keep this stage explicitly
    // indeterminate instead of deriving a percentage from bytes or elapsed
    // time, then report its boundary once parsing/import has returned.
    report(PmxLoadStage::ParsingPmx, None, None);
    let document = parse_pmx(&bytes).map_err(|error| error.to_string())?;
    let model = import_pmx(
        document,
        &PmxImportContext::with_source(source.bevy_source().clone()),
    )
    .model;
    report(PmxLoadStage::ParsingPmx, Some(1), Some(1));

    let (bounds_min, bounds_max) = bounds_for_model(&model).ok_or_else(|| {
        format!(
            "{} contains no vertices",
            source.identity().path().display()
        )
    })?;
    let (textures, warnings) = load_textures(source, model.texture_paths(), |completed, total| {
        report(PmxLoadStage::LoadingTextures, Some(completed), Some(total));
    });
    let primitive_splits = build_primitive_splits(&model);
    let info = scene_info(
        source.identity(),
        &model,
        warnings,
        &request.existing_slot_ids,
        &primitive_splits,
    );

    Ok(PreparedPmxScene {
        info,
        model,
        primitive_splits,
        textures,
        bounds_min,
        bounds_max,
    })
}

pub(crate) struct SpawnedPmxScene {
    entities: Vec<Entity>,
    images: Vec<Handle<Image>>,
    meshes: Vec<Handle<Mesh>>,
    primitive_entities: Vec<Option<Entity>>,
    primitive_material_indices: Vec<Option<usize>>,
    split_geometry: PmxMeshGeometry,
    primitive_ranges: Vec<PrimitiveRange>,
    pub(crate) materials: Vec<Handle<CharmeMaterial>>,
    pub(crate) material_slot_ids: Vec<MaterialSlotId>,
}

impl SpawnedPmxScene {
    pub(crate) fn material_for_slot(
        &self,
        slot_id: MaterialSlotId,
    ) -> Option<&Handle<CharmeMaterial>> {
        self.material_slot_ids
            .iter()
            .position(|candidate| *candidate == slot_id)
            .and_then(|index| self.materials.get(index))
    }

    pub(crate) fn split_primitives_by_connectivity(
        &mut self,
        app: &mut App,
        primitive_indices: &[usize],
    ) -> bool {
        let mut primitive_indices = primitive_indices.to_vec();
        primitive_indices.sort_unstable();
        primitive_indices.dedup();

        let mut changed = false;
        for primitive_index in primitive_indices {
            let Some(Some(original_entity)) = self.primitive_entities.get(primitive_index) else {
                continue;
            };
            let Some(material_index) = self
                .primitive_material_indices
                .get(primitive_index)
                .and_then(Option::as_ref)
                .copied()
            else {
                continue;
            };
            let Some(material) = self.materials.get(material_index).cloned() else {
                continue;
            };
            let Some(range) = self.primitive_ranges.get(primitive_index).copied() else {
                continue;
            };
            let Some(component_indices) = split_primitive(
                &self.split_geometry.indices,
                self.split_geometry.positions.len(),
                range,
            )
            .ok()
            .filter(|split| split.components.len() > 1)
            .map(|split| {
                split
                    .components
                    .into_iter()
                    .map(|component| component.indices)
                    .collect::<Vec<_>>()
            }) else {
                continue;
            };
            let transform = app
                .world()
                .get::<Transform>(*original_entity)
                .cloned()
                .unwrap_or_default();

            for (component_index, indices) in component_indices.into_iter().enumerate() {
                let mesh = app
                    .world_mut()
                    .resource_mut::<Assets<Mesh>>()
                    .add(mesh_for_indices(&self.split_geometry, &indices));
                self.meshes.push(mesh.clone());
                let entity = app
                    .world_mut()
                    .spawn((
                        Name::new(format!(
                            "PMX Primitive {primitive_index} Component {component_index}"
                        )),
                        Mesh3d(mesh),
                        MeshMaterial3d(material.clone()),
                        transform,
                    ))
                    .id();
                self.entities.push(entity);
            }

            let _ = app.world_mut().despawn(*original_entity);
            self.entities.retain(|entity| entity != original_entity);
            self.primitive_entities[primitive_index] = None;
            changed = true;
        }
        changed
    }
}

impl SpawnedPmxScene {
    pub fn despawn(self, app: &mut App) {
        for entity in self.entities {
            let _ = app.world_mut().despawn(entity);
        }
        for handle in self.materials {
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .remove(handle.id());
        }
        for handle in self.meshes {
            app.world_mut()
                .resource_mut::<Assets<Mesh>>()
                .remove(handle.id());
        }
        for handle in self.images {
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .remove(handle.id());
        }
    }
}

pub(crate) fn spawn_pmx_scene(
    app: &mut App,
    prepared: &PreparedPmxScene,
    mut report: impl FnMut(usize, usize),
) -> SpawnedPmxScene {
    let can_spawn_primitive = prepared.model.primitives().iter().any(|primitive| {
        prepared
            .model
            .material_records()
            .get(primitive.material_index)
            .is_some()
    });
    let total = prepared.textures.len()
        + prepared.model.material_records().len()
        + prepared.model.primitives().len()
        + usize::from(!can_spawn_primitive);
    let mut completed = 0;
    report(completed, total);

    let mut texture_handles = Vec::with_capacity(prepared.textures.len());
    for texture in &prepared.textures {
        texture_handles.push(
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(texture.image.clone()),
        );
        completed += 1;
        report(completed, total);
    }
    let texture_has_alpha = prepared
        .textures
        .iter()
        .map(|texture| texture.has_alpha)
        .collect::<Vec<_>>();
    let center = (prepared.bounds_min + prepared.bounds_max) * 0.5;
    let transform =
        Transform::from_translation(Vec3::new(-center.x, -prepared.bounds_min.y, -center.z));
    let mut entities = Vec::new();
    let mut mesh_handles = Vec::new();
    let mut primitive_entities = vec![None; prepared.model.primitives().len()];
    let mut primitive_material_indices = vec![None; prepared.model.primitives().len()];
    // Keep one material asset per PMX slot. Apart from avoiding duplicate
    // assets for slots used by multiple primitives, this gives the renderer a
    // stable slot-to-material mapping for material-ball previews.
    let mut material_handles = Vec::with_capacity(prepared.model.material_records().len());
    for record in prepared.model.material_records() {
        material_handles.push(
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .add(material_for_record(
                    record,
                    &texture_handles,
                    &texture_has_alpha,
                )),
        );
        completed += 1;
        report(completed, total);
    }

    for (primitive_index, primitive) in prepared.model.primitives().iter().enumerate() {
        if let (Some(record), Some(material)) = (
            prepared
                .model
                .material_records()
                .get(primitive.material_index),
            material_handles.get(primitive.material_index).cloned(),
        ) {
            primitive_material_indices[primitive_index] = Some(primitive.material_index);
            let mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(prepared.model.geometry().to_mesh_for_primitive(*primitive));
            mesh_handles.push(mesh.clone());
            let entity = app
                .world_mut()
                .spawn((
                    Name::new(format!(
                        "PMX Primitive {primitive_index} ({})",
                        record.material.name
                    )),
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    transform,
                ))
                .id();
            primitive_entities[primitive_index] = Some(entity);
            entities.push(entity);
        }
        completed += 1;
        report(completed, total);
    }

    if entities.is_empty() {
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(prepared.model.geometry().to_mesh());
        let material = app
            .world_mut()
            .resource_mut::<Assets<CharmeMaterial>>()
            .add(CharmeMaterial::default());
        mesh_handles.push(mesh.clone());
        material_handles.push(material.clone());
        entities.push(
            app.world_mut()
                .spawn((
                    Name::new("PMX Model"),
                    Mesh3d(mesh),
                    MeshMaterial3d(material),
                    transform,
                ))
                .id(),
        );
        completed += 1;
        report(completed, total);
    }

    SpawnedPmxScene {
        entities,
        images: texture_handles,
        meshes: mesh_handles,
        primitive_entities,
        primitive_material_indices,
        split_geometry: prepared.model.geometry().clone(),
        primitive_ranges: prepared
            .model
            .primitives()
            .iter()
            .map(|primitive| PrimitiveRange::new(primitive.index_start, primitive.index_count))
            .collect(),
        material_slot_ids: prepared
            .info
            .material_slots()
            .iter()
            .map(PmxMaterialSlot::id)
            .collect(),
        materials: material_handles,
    }
}

fn mesh_for_indices(geometry: &PmxMeshGeometry, indices: &[u32]) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, geometry.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, geometry.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, geometry.uvs.clone());
    mesh.insert_indices(Indices::U32(indices.to_vec()));
    mesh
}

fn scene_info(
    source: &PmxSourceIdentity,
    model: &Pmx,
    warnings: Vec<String>,
    existing_slot_ids: &[(u32, MaterialSlotId)],
    primitive_splits: &[Option<PrimitiveSplit>],
) -> PmxSceneInfo {
    let name = model
        .raw_document()
        .and_then(|document| {
            (!document.header.model_name.is_empty())
                .then(|| document.header.model_name.clone())
                .or_else(|| {
                    (!document.header.model_name_english.is_empty())
                        .then(|| document.header.model_name_english.clone())
                })
        })
        .or_else(|| {
            source
                .archive_entry()
                .map(Path::new)
                .or_else(|| Some(source.path()))
                .and_then(Path::file_stem)
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "PMX Model".to_owned());
    let material_slots = model
        .material_records()
        .iter()
        .enumerate()
        .map(|(index, record)| PmxMaterialSlot {
            id: existing_slot_ids
                .iter()
                .find(|(source_index, _)| *source_index == index as u32)
                .map(|(_, id)| *id)
                .unwrap_or_else(MaterialSlotId::new),
            index,
            name: record.material.name.clone(),
            english_name: record.material.name_english.clone(),
            diffuse_texture: texture_path(model, record.material.texture_index),
            sphere_texture: texture_path(model, record.material.sphere_texture_index),
            toon_texture: if record.material.toon_sharing {
                (record.material.toon_texture_index >= 0)
                    .then(|| format!("shared_toon_{:02}", record.material.toon_texture_index + 1))
            } else {
                texture_path(model, record.material.toon_texture_index)
            },
        })
        .collect::<Vec<_>>();
    let primitives = model
        .primitives()
        .iter()
        .enumerate()
        .map(|(index, primitive)| PmxPrimitiveInfo {
            index,
            index_count: primitive.index_count,
            material_slot_id: material_slots
                .get(primitive.material_index)
                .map(PmxMaterialSlot::id),
            components: primitive_splits
                .get(index)
                .and_then(Option::as_ref)
                .map(primitive_component_infos)
                .unwrap_or_default(),
        })
        .collect();

    PmxSceneInfo {
        source: source.clone(),
        name,
        vertex_count: model.geometry().positions.len(),
        index_count: model.geometry().indices.len(),
        material_slots,
        primitives,
        warnings,
    }
}

fn build_primitive_splits(model: &Pmx) -> Vec<Option<PrimitiveSplit>> {
    let geometry = model.geometry();
    model
        .primitives()
        .iter()
        .map(|primitive| {
            split_primitive(
                &geometry.indices,
                geometry.positions.len(),
                PrimitiveRange::new(primitive.index_start, primitive.index_count),
            )
            .ok()
        })
        .collect()
}

fn primitive_component_infos(split: &PrimitiveSplit) -> Vec<PmxPrimitiveComponentInfo> {
    split
        .components
        .iter()
        .enumerate()
        .map(|(index, component)| PmxPrimitiveComponentInfo {
            index,
            triangle_count: component.triangle_count(),
            index_count: component.index_count(),
            vertex_count: component.vertex_indices.len(),
        })
        .collect()
}

fn texture_path(model: &Pmx, index: i32) -> Option<String> {
    (index >= 0)
        .then_some(index as usize)
        .and_then(|index| model.texture_paths().get(index))
        .map(|path| path.original.clone())
}

fn load_textures(
    source: &dyn PmxInputSource,
    paths: &[PmxResolvedPath],
    mut report: impl FnMut(usize, usize),
) -> (Vec<DecodedTexture>, Vec<String>) {
    let mut textures = Vec::with_capacity(paths.len());
    let mut warnings = Vec::new();
    let total = paths.len();
    report(0, total);

    for (completed, path) in paths.iter().enumerate() {
        match load_texture(source, path) {
            Ok(texture) => textures.push(texture),
            Err(error) => {
                warnings.push(format!(
                    "failed to load texture {} (resolved to {}): {error}",
                    path.original,
                    path.location()
                ));
                textures.push(DecodedTexture {
                    image: placeholder_texture(),
                    has_alpha: false,
                });
            }
        }
        report(completed + 1, total);
    }

    (textures, warnings)
}

fn load_texture(
    source: &dyn PmxInputSource,
    path: &PmxResolvedPath,
) -> Result<DecodedTexture, io::Error> {
    let location = path.location();
    let bytes = source.read_texture_bytes(path).map_err(|error| {
        io::Error::other(format!(
            "failed to read {} through the PMX source: {error}",
            path.original
        ))
    })?;
    let extension = location.extension().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("texture {location} does not have a file extension"),
        )
    })?;
    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension(extension.to_ascii_lowercase().as_str()),
        CompressedImageFormats::all(),
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let has_alpha = image_has_alpha(&image);
    Ok(DecodedTexture { image, has_alpha })
}

fn material_for_record(
    record: &PmxMaterialRecord,
    textures: &[Handle<Image>],
    texture_has_alpha: &[bool],
) -> CharmeMaterial {
    let [red, green, blue, alpha] = record.material.diffuse;
    let texture_index =
        (record.material.texture_index >= 0).then_some(record.material.texture_index as usize);
    let base_color_texture = texture_index.and_then(|index| textures.get(index).cloned());
    let has_texture_alpha = texture_index
        .and_then(|index| texture_has_alpha.get(index).copied())
        .unwrap_or(false);

    CharmeMaterial {
        parameters: CharmeMaterialParams::with_tint([red, green, blue, alpha]),
        base_color_texture,
        alpha_mode: if alpha < 0.999 {
            AlphaMode::Blend
        } else if has_texture_alpha {
            AlphaMode::AlphaToCoverage
        } else {
            AlphaMode::Opaque
        },
    }
}

fn bounds_for_model(model: &Pmx) -> Option<(Vec3, Vec3)> {
    let first = Vec3::from(*model.geometry().positions.first()?);
    let mut minimum = first;
    let mut maximum = first;
    for position in &model.geometry().positions[1..] {
        let position = Vec3::from(*position);
        minimum = minimum.min(position);
        maximum = maximum.max(position);
    }
    Some((minimum, maximum))
}

fn image_has_alpha(image: &Image) -> bool {
    matches!(
        image.texture_descriptor.format,
        TextureFormat::Rgba8Unorm
            | TextureFormat::Rgba8UnormSrgb
            | TextureFormat::Bgra8Unorm
            | TextureFormat::Bgra8UnormSrgb
            | TextureFormat::Rgba16Float
            | TextureFormat::Rgba32Float
    )
}

fn placeholder_texture() -> Image {
    Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 0, 255, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Dir3, Ray3d};
    use bevy_pmx::PmxPrimitive;

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
    fn coplanar_triangle_diagonals_are_not_silhouette_boundaries() {
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

        assert_eq!(edges.len(), 5);
        assert_eq!(edges.iter().filter(|edge| edge.faces.len() == 2).count(), 1);
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
