use std::{io, path::Path};

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::{
        AlphaMode, App, Assets, Entity, Handle, Image, Mesh, Mesh3d, MeshMaterial3d, Name,
        Transform, Vec3,
    },
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_pmx::{Pmx, PmxImportContext, PmxMaterialRecord, PmxResolvedPath, import_pmx, parse_pmx};
use charme_bevy::{CharmeMaterial, CharmeMaterialParams};
use charme_core::MaterialSlotId;
use charme_geometry::{PrimitiveRange, PrimitiveSplit, split_primitive};

#[cfg(test)]
use crate::selection::{
    PrimitiveComponentSelectionGeometry, PrimitiveSelectionGeometry, SelectionGeometry,
    selection_edges, selection_face,
};

use crate::{
    PmxLoadStage,
    overlay::PreviewOverlays,
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
    pub(crate) model: Pmx,
    pub(crate) primitive_splits: Vec<Option<PrimitiveSplit>>,
    textures: Vec<DecodedTexture>,
    pub(crate) bounds_min: Vec3,
    pub(crate) bounds_max: Vec3,
}

impl PreparedPmxScene {
    pub fn normalized_bounds(&self) -> (Vec3, Vec3) {
        let center = (self.bounds_min + self.bounds_max) * 0.5;
        let translation = Vec3::new(-center.x, -self.bounds_min.y, -center.z);
        (self.bounds_min + translation, self.bounds_max + translation)
    }
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
    primitive_component_counts: Vec<usize>,
    overlays: PreviewOverlays,
    pub(crate) materials: Vec<Handle<CharmeMaterial>>,
    default_material_parameters: Vec<CharmeMaterialParams>,
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

    pub(crate) fn material_state_for_slot(
        &self,
        slot_id: MaterialSlotId,
    ) -> Option<(&Handle<CharmeMaterial>, CharmeMaterialParams)> {
        let index = self
            .material_slot_ids
            .iter()
            .position(|candidate| *candidate == slot_id)?;
        Some((
            self.materials.get(index)?,
            *self.default_material_parameters.get(index)?,
        ))
    }

    pub(crate) fn split_primitives_by_connectivity(
        &mut self,
        app: &mut App,
        primitive_indices: &[usize],
    ) -> bool {
        self.overlays.show_connectivity(
            app,
            &self.primitive_entities,
            &self.primitive_component_counts,
            primitive_indices,
        )
    }
}

impl SpawnedPmxScene {
    pub fn despawn(self, app: &mut App) {
        self.overlays.despawn(app);
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
    // Keep one material asset per PMX slot. Apart from avoiding duplicate
    // assets for slots used by multiple primitives, this gives the renderer a
    // stable slot-to-material mapping for material-ball previews.
    let mut material_handles = Vec::with_capacity(prepared.model.material_records().len());
    let mut default_material_parameters =
        Vec::with_capacity(prepared.model.material_records().len());
    for record in prepared.model.material_records() {
        let material = material_for_record(record, &texture_handles, &texture_has_alpha);
        default_material_parameters.push(material.parameters);
        material_handles.push(
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .add(material),
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
        default_material_parameters.push(CharmeMaterialParams::default());
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
        primitive_component_counts: prepared
            .primitive_splits
            .iter()
            .map(|split| split.as_ref().map_or(0, |split| split.components.len()))
            .collect(),
        overlays: PreviewOverlays::default(),
        material_slot_ids: prepared
            .info
            .material_slots()
            .iter()
            .map(PmxMaterialSlot::id)
            .collect(),
        materials: material_handles,
        default_material_parameters,
    }
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
