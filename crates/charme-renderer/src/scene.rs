use std::{
    io,
    path::{Path, PathBuf},
};

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::{
        AlphaMode, App, Assets, Entity, Handle, Image, Mesh, Mesh3d, MeshMaterial3d, Name,
        Transform, Vec3,
    },
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_pmx::{
    Pmx, PmxImportContext, PmxMaterialRecord, PmxResolvedPath, PmxSource, PmxSourceLocation,
    import_pmx, parse_pmx,
};
use charme_bevy::{CharmeMaterial, CharmeMaterialParams};
use charme_core::MaterialSlotId;

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

/// UI-facing summary of a loaded PMX scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PmxSceneInfo {
    path: PathBuf,
    name: String,
    vertex_count: usize,
    index_count: usize,
    material_slots: Vec<PmxMaterialSlot>,
    warnings: Vec<String>,
}

impl PmxSceneInfo {
    /// Returns the source PMX path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the model name, falling back to the source file name.
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

    /// Returns recoverable import warnings, such as missing textures.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

pub(crate) struct PreparedPmxScene {
    pub info: PmxSceneInfo,
    model: Pmx,
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

struct DecodedTexture {
    image: Image,
    has_alpha: bool,
}

pub(crate) fn prepare_pmx_scene(
    path: &Path,
    existing_slot_ids: &[(u32, MaterialSlotId)],
) -> Result<PreparedPmxScene, String> {
    let source = PmxSource::folder(path.parent().unwrap_or_else(|| Path::new(".")));
    let location = PmxSourceLocation::disk(path.to_path_buf());
    let bytes = source.read_bytes(&location).map_err(|error| {
        format!(
            "failed to read PMX file {} (resolved to {location}): {error}",
            path.display()
        )
    })?;
    let document = parse_pmx(&bytes).map_err(|error| error.to_string())?;
    let model = import_pmx(document, &PmxImportContext::with_source(source.clone())).model;

    let (bounds_min, bounds_max) = bounds_for_model(&model)
        .ok_or_else(|| format!("{} contains no vertices", path.display()))?;
    let (textures, warnings) = load_textures(&source, model.texture_paths());
    let info = scene_info(path, &model, warnings, existing_slot_ids);

    Ok(PreparedPmxScene {
        info,
        model,
        textures,
        bounds_min,
        bounds_max,
    })
}

pub(crate) struct SpawnedPmxScene {
    entities: Vec<Entity>,
    images: Vec<Handle<Image>>,
    meshes: Vec<Handle<Mesh>>,
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

pub(crate) fn spawn_pmx_scene(app: &mut App, prepared: &PreparedPmxScene) -> SpawnedPmxScene {
    let texture_handles = prepared
        .textures
        .iter()
        .map(|texture| {
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(texture.image.clone())
        })
        .collect::<Vec<_>>();
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
    // Keep one material asset per PMX slot. Apart from avoiding duplicate
    // assets for slots used by multiple primitives, this gives the renderer a
    // stable slot-to-material mapping for material-ball previews.
    let mut material_handles = prepared
        .model
        .material_records()
        .iter()
        .map(|record| {
            app.world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .add(material_for_record(
                    record,
                    &texture_handles,
                    &texture_has_alpha,
                ))
        })
        .collect::<Vec<_>>();

    for (primitive_index, primitive) in prepared.model.primitives().iter().enumerate() {
        let Some(record) = prepared
            .model
            .material_records()
            .get(primitive.material_index)
        else {
            continue;
        };
        let Some(material) = material_handles.get(primitive.material_index).cloned() else {
            continue;
        };
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
                MeshMaterial3d(material),
                transform,
            ))
            .id();
        entities.push(entity);
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
    }

    SpawnedPmxScene {
        entities,
        images: texture_handles,
        meshes: mesh_handles,
        material_slot_ids: prepared
            .info
            .material_slots()
            .iter()
            .map(PmxMaterialSlot::id)
            .collect(),
        materials: material_handles,
    }
}

fn scene_info(
    path: &Path,
    model: &Pmx,
    warnings: Vec<String>,
    existing_slot_ids: &[(u32, MaterialSlotId)],
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
            path.file_stem()
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
        .collect();

    PmxSceneInfo {
        path: path.to_path_buf(),
        name,
        vertex_count: model.geometry().positions.len(),
        index_count: model.geometry().indices.len(),
        material_slots,
        warnings,
    }
}

fn texture_path(model: &Pmx, index: i32) -> Option<String> {
    (index >= 0)
        .then_some(index as usize)
        .and_then(|index| model.texture_paths().get(index))
        .map(|path| path.original.clone())
}

fn load_textures(
    source: &PmxSource,
    paths: &[PmxResolvedPath],
) -> (Vec<DecodedTexture>, Vec<String>) {
    let mut textures = Vec::with_capacity(paths.len());
    let mut warnings = Vec::new();

    for path in paths {
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
    }

    (textures, warnings)
}

fn load_texture(source: &PmxSource, path: &PmxResolvedPath) -> Result<DecodedTexture, io::Error> {
    let location = path.location();
    let bytes = source.read_bytes(location).map_err(|error| {
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
    _textures: &[Handle<Image>],
    _texture_has_alpha: &[bool],
) -> CharmeMaterial {
    let [red, green, blue, alpha] = record.material.diffuse;
    CharmeMaterial {
        parameters: CharmeMaterialParams::with_tint([red, green, blue, alpha]),
        alpha_mode: if alpha < 0.999 {
            AlphaMode::Blend
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
