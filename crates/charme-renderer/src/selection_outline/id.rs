//! Object-ID map rendering for the selected-object outline.
//!
//! Selected primitives are rendered into a dedicated offscreen `Rgba8Unorm`
//! texture through a second camera. Each primitive carries a constant,
//! per-vertex ID (baked into the mesh color attribute) that survives the ID
//! material's pixel output; the 16-bit value packs a 2-bit color class and a
//! 14-bit object ID exactly like Blender's `outline_id_pack`.

use bevy::{
    app::App,
    asset::{Asset, Assets, Handle, RenderAssetUsages, load_internal_asset, uuid_handle},
    camera::{
        Camera, Camera3d, ClearColorConfig, Projection, RenderTarget, primitives::Frustum,
        visibility::RenderLayers,
    },
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    ecs::{
        entity::Entity,
        query::{Changed, Or, With, Without},
        resource::Resource,
        system::Query,
    },
    image::Image,
    math::Vec4,
    pbr::{Material, MaterialPlugin},
    prelude::{AlphaMode, Color, Component, Mesh},
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureUsages},
        view::Msaa,
    },
    shader::{Shader, ShaderRef},
    transform::components::{GlobalTransform, Transform},
};

use super::SELECTION_ID_LAYER;
use crate::{OutputSize, backend::MainPreviewCamera};

/// Stable handle for the embedded ID material shader.
const OUTLINE_ID_MATERIAL_SHADER: Handle<Shader> =
    uuid_handle!("57e2f0a1-4b6c-4d8e-9f0a-1b2c3d4e5f60");

/// Registers the ID material asset and its embedded shader.
pub(crate) fn install_outline_id_material(app: &mut App) {
    load_internal_asset!(
        app,
        OUTLINE_ID_MATERIAL_SHADER,
        "selection_outline_id.wgsl",
        Shader::from_wgsl
    );
    app.add_plugins(MaterialPlugin::<OutlineIdMaterial>::default());
}

/// Marker for the selection ID camera.
#[derive(Component, bevy::render::extract_component::ExtractComponent, Clone)]
pub(crate) struct SelectionIdCamera;

/// The ID camera and its offscreen ID render target.
#[derive(Resource, Debug, Clone)]
pub(crate) struct SelectionOutline {
    pub(crate) id_target: Handle<Image>,
    pub(crate) id_camera: Entity,
}

impl Default for SelectionOutline {
    fn default() -> Self {
        Self {
            id_target: Handle::default(),
            id_camera: Entity::PLACEHOLDER,
        }
    }
}

/// Opaque material whose fragment shader writes the object ID encoded in the
/// mesh color attribute into the ID target's red/green channels.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub(crate) struct OutlineIdMaterial {
    /// Reserved binding keeping the material bind group non-empty; the ID
    /// shader never reads it.
    #[uniform(0)]
    _reserved: Vec4,
}

impl Material for OutlineIdMaterial {
    fn fragment_shader() -> ShaderRef {
        OUTLINE_ID_MATERIAL_SHADER.clone().into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }
}

/// Encodes the 2-bit color class (`selected` for now) and the 14-bit object
/// ID into the 16-bit value written to the ID texture.
const fn pack_outline_id(outline_id: u32, object_id: u32) -> u32 {
    (outline_id << 14) | (object_id & 0x3FFF)
}

/// Returns the object ID assigned to a primitive. ID 0 is reserved for the
/// background.
pub(crate) const fn outline_id(primitive_index: usize) -> u32 {
    pack_outline_id(1, primitive_index as u32 + 1)
}

/// Bakes the object ID for a primitive into a mesh as a constant vertex
/// color. The ID material reads `input.color.r` and recovers the ID through
/// `u32(...)`, which is exact for IDs below 2^24.
pub(crate) fn bake_outline_id(mesh: &mut Mesh, primitive_index: usize) {
    let id = outline_id(primitive_index);
    let values =
        std::iter::repeat_n([id as f32, 0.0, 0.0, 1.0], mesh.count_vertices()).collect::<Vec<_>>();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, values);
}

/// Spawns the ID camera and its render target.
///
/// The camera state defaults are overwritten by [`sync_selection_id_camera`]
/// on the first update, so the initial projection/transform are irrelevant.
pub(crate) fn spawn_selection_outline(app: &mut App, size: OutputSize) -> SelectionOutline {
    let id_target = add_selection_id_target(app, size);
    let world = app.world_mut();

    let id_camera = world
        .spawn((
            SelectionIdCamera,
            Camera3d::default(),
            Tonemapping::None,
            // The ID texture carries exact integer bits; debanding dither and
            // tonemapping would corrupt them.
            DebandDither::Disabled,
            Msaa::Off,
            RenderLayers::layer(SELECTION_ID_LAYER),
            RenderTarget::Image(id_target.clone().into()),
            Camera {
                order: 0,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                is_active: true,
                ..Default::default()
            },
            Projection::default(),
            Transform::IDENTITY,
        ))
        .id();

    SelectionOutline {
        id_target,
        id_camera,
    }
}

impl SelectionOutline {
    /// Recreates the ID target after a viewport resize.
    pub(crate) fn resize(&mut self, app: &mut App, size: OutputSize) {
        let (removed, added) = {
            let world = app.world_mut();
            let removed = world
                .resource_mut::<Assets<Image>>()
                .remove(self.id_target.id());
            let added = world
                .resource_mut::<Assets<Image>>()
                .add(new_selection_id_image(size));
            (removed, added)
        };
        drop(removed);
        self.id_target = added.clone();
        if let Some(mut target) = app.world_mut().get_mut::<RenderTarget>(self.id_camera) {
            *target = RenderTarget::Image(added.into());
        }
    }
}

/// Copies the main camera state onto the ID camera so both views line up.
#[allow(clippy::type_complexity)]
pub(crate) fn sync_selection_id_camera(
    main: Query<
        (&Projection, &GlobalTransform, &Camera),
        (
            With<MainPreviewCamera>,
            Without<SelectionIdCamera>,
            Or<(
                Changed<Projection>,
                Changed<GlobalTransform>,
                Changed<Camera>,
            )>,
        ),
    >,
    mut id: Query<
        (
            &mut Projection,
            &mut GlobalTransform,
            &mut Camera,
            &mut Frustum,
        ),
        (With<SelectionIdCamera>, Without<MainPreviewCamera>),
    >,
) {
    let Ok((projection, transform, camera)) = main.single() else {
        return;
    };
    let Ok((mut id_projection, mut id_transform, mut id_camera, mut id_frustum)) = id.single_mut()
    else {
        return;
    };
    *id_projection = projection.clone();
    *id_transform = *transform;
    id_camera.is_active = camera.is_active;
    // Keep the frustum in sync with the copied camera state. `update_frusta`
    // only recomputes it when the ID camera's own components change, which
    // would leave a stale frustum (and cull the ID proxies) whenever the main
    // camera stops moving.
    *id_frustum = projection.compute_frustum(transform);
}

fn new_selection_id_image(size: OutputSize) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Linear storage: the ID pass writes exact integer bits and the
        // composite node reads them back without an sRGB round trip.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    image
}

fn add_selection_id_target(app: &mut App, size: OutputSize) -> Handle<Image> {
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .add(new_selection_id_image(size))
}
