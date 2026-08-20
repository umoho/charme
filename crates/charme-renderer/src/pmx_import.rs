use std::{io, path::Path};

use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::{AlphaMode, Handle, Image, Vec3},
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_pmx::{Pmx, PmxImportContext, PmxMaterialRecord, PmxResolvedPath, import_pmx, parse_pmx};
use charme_bevy::{CharmeMaterial, CharmeMaterialParams};
use charme_core::MaterialSlotId;
use charme_geometry::{PrimitiveRange, PrimitiveSplit, split_primitive};

use crate::{
    PmxLoadStage,
    scene::{PmxMaterialSlot, PmxPrimitiveComponentInfo, PmxPrimitiveInfo, PmxSceneInfo},
    source::{PmxInputSource, PmxSourceIdentity, ResolvedPmxLoadRequest},
};

pub(crate) struct PreparedPmxScene {
    pub info: PmxSceneInfo,
    pub(crate) model: Pmx,
    pub(crate) primitive_splits: Vec<Option<PrimitiveSplit>>,
    pub(crate) textures: Vec<DecodedTexture>,
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

pub(crate) struct DecodedTexture {
    pub(crate) image: Image,
    pub(crate) has_alpha: bool,
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

pub(crate) fn build_primitive_splits(model: &Pmx) -> Vec<Option<PrimitiveSplit>> {
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

pub(crate) fn primitive_component_infos(split: &PrimitiveSplit) -> Vec<PmxPrimitiveComponentInfo> {
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

pub(crate) fn material_for_record(
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
