//! Selection wireframe rendering.
//!
//! The wireframe mimics Blender's selected-object wire overlay:
//!
//! * Back-face wireframe is culled and interior mesh lines stay silhouette-only
//!   (the CPU-side edge classification in [`update_selection_wire`]).
//! * The wireframe is never occluded by other objects, but is occluded by the
//!   selected object itself.
//!
//! This is achieved with a dedicated offscreen "mask" camera:
//!
//! * The mask camera runs a custom render schedule containing only the depth
//!   prepass (writing the selected object's depth into the mask view depth
//!   buffer), an alpha-mask-only pass drawing the wireframe line quads, and
//!   the upscaling blit that writes the mask image.
//! * Because no other geometry renders into the mask camera, the hardware
//!   depth test of the line pipeline provides self-occlusion for free while
//!   other objects can never occlude the wireframe.
//! * A fullscreen composite node on the main camera blends the mask image
//!   over the final frame.

use bevy::{
    app::{App, PostUpdate},
    asset::{Asset, Handle, RenderAssetUsages, embedded_path, load_embedded_asset},
    camera::{
        Camera, Camera3d, ClearColorConfig, Projection, RenderTarget, Viewport,
        visibility::{NoFrustumCulling, RenderLayers},
    },
    core_pipeline::{
        FullscreenShader,
        core_3d::AlphaMask3d,
        prepass::{DepthPrepass, node::early_prepass},
        schedule::{Core3d, Core3dSystems},
        tonemapping::{Tonemapping, tonemapping},
        upscaling::upscaling,
    },
    ecs::{
        change_detection::DetectChanges,
        query::With,
        schedule::{IntoScheduleConfigs, Schedule, ScheduleBuildSettings, ScheduleLabel},
        world::World,
    },
    image::Image,
    log::error,
    math::Vec3,
    mesh::{Mesh, MeshVertexAttribute},
    pbr::{Material, MaterialPlugin, MeshMaterial3d},
    prelude::{
        AlphaMode, Assets, Color, Commands, Component, Entity, GlobalTransform, Local, Mesh3d,
        Query, Res, ResMut, Resource, Transform, Visibility,
    },
    reflect::TypePath,
    render::{
        GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems,
        camera::{CameraRenderGraph, ExtractedCamera},
        extract_component::ExtractComponentPlugin,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_phase::ViewBinnedRenderPhases,
        render_resource::{
            AsBindGroup, BindGroup, BindGroupEntries, BindGroupLayoutDescriptor,
            BindGroupLayoutEntries, CachedRenderPipelineId, ColorTargetState, ColorWrites,
            Extent3d, FilterMode, FragmentState, LoadOp, MultisampleState, Operations,
            PipelineCache, PrimitiveState, PrimitiveTopology, RenderPassColorAttachment,
            RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType,
            SamplerDescriptor, ShaderStages, SpecializedRenderPipeline, SpecializedRenderPipelines,
            StoreOp, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
            TextureViewId, VertexFormat,
            binding_types::{sampler, texture_2d},
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::GpuImage,
        view::{ExtractedView, ViewDepthTexture, ViewTarget},
    },
    shader::{Shader, ShaderRef},
    transform::TransformSystems,
    utils::default,
};

use crate::{
    OutputSize,
    backend::MainPreviewCamera,
    selection::{SelectionFace, SelectionGeometry},
};

/// Render layer of the selection mask camera.
pub(crate) const SELECTION_WIRE_LAYER: usize = 1;

/// Marker for the selection mask camera.
#[derive(Component)]
struct SelectionMaskCamera;

/// Marker for the wireframe line mesh entity.
#[derive(Component)]
struct SelectionWireLineEntity;

/// Marker for the main camera's composite pipeline.
#[derive(Component)]
struct ViewSelectionWireCompositePipeline(CachedRenderPipelineId);

/// Whether the composite pass has wireframe lines to draw this frame.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionWireActive(bool);

/// The main-world handle of the wireframe mask image, extracted for the
/// composite node.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionWireMaskHandle(pub(crate) Handle<Image>);

/// Main-world handles describing the selection wireframe setup.
#[derive(Resource, Debug, Clone)]
pub(crate) struct SelectionWire {
    pub(crate) mask_target: Handle<Image>,
    pub(crate) mask_camera: Entity,
    /// Retained for debugging; systems find the line entity by its marker.
    pub(crate) _line_entity: Entity,
    pub(crate) line_mesh: Handle<Mesh>,
    /// Retained so the material asset stays alive.
    pub(crate) _line_material: Handle<SelectionWireMaterial>,
}

impl Default for SelectionWire {
    fn default() -> Self {
        Self {
            mask_target: Handle::default(),
            mask_camera: Entity::PLACEHOLDER,
            _line_entity: Entity::PLACEHOLDER,
            line_mesh: Handle::default(),
            _line_material: Handle::default(),
        }
    }
}

/// Wireframe line material.
///
/// The mask camera's schedule skips the opaque and transparent passes, so only
/// alpha-mask materials reach the mask color target. PMX materials never use
/// [`AlphaMode::Mask`], which makes this phase exclusive to the wireframe.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub(crate) struct SelectionWireMaterial {
    /// Unused binding keeping the material bind group layout non-empty.
    #[uniform(0)]
    _unused: u32,
}

impl Material for SelectionWireMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(embedded_path!("selection_wire_lines.wgsl"))
                .with_source("embedded"),
        )
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            bevy::asset::AssetPath::from_path_buf(embedded_path!("selection_wire_lines.wgsl"))
                .with_source("embedded"),
        )
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.5)
    }

    fn enable_prepass() -> bool {
        // The line mesh uses a custom vertex layout that the default prepass
        // shader does not support. The mask camera's depth prepass only needs
        // the selected object's depth, so lines skip it entirely.
        false
    }
}

/// Schedule label for the selection mask camera.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct SelectionWireMaskGraph;

const SELECTION_WIRE_POSITION_B: MeshVertexAttribute =
    MeshVertexAttribute::new("Vertex_Position_B", 1, VertexFormat::Float32x3);

const SELECTION_WIRE_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("Vertex_Corner", 2, VertexFormat::Uint32);

/// Registers the selection wireframe systems, resources, and render schedule.
///
/// Must be called before `App::finish` so the render app picks up the mask
/// schedule and the composite node.
pub(crate) fn install_selection_wire(app: &mut App) {
    app.add_plugins((
        MaterialPlugin::<SelectionWireMaterial>::default(),
        ExtractResourcePlugin::<SelectionWireActive>::default(),
        ExtractResourcePlugin::<SelectionWireMaskHandle>::default(),
        ExtractComponentPlugin::<MainPreviewCamera>::default(),
    ))
    .init_resource::<SelectionWireActive>()
    .init_resource::<SelectionWireMaskHandle>()
    .init_resource::<SelectionWire>()
    .add_systems(
        PostUpdate,
        (
            sync_selection_mask_camera.after(TransformSystems::Propagate),
            update_selection_wire,
        ),
    );

    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .add_schedule(selection_wire_mask_schedule())
            .init_gpu_resource::<SpecializedRenderPipelines<SelectionWireCompositePipeline>>()
            .add_systems(RenderStartup, init_selection_wire_composite_pipeline)
            .add_systems(
                Core3d,
                selection_wire_composite
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
            )
            .add_systems(
                Render,
                prepare_view_selection_wire_composite_pipelines.in_set(RenderSystems::Prepare),
            );
    }

    bevy::asset::embedded_asset!(app, "selection_wire_lines.wgsl");
    bevy::asset::embedded_asset!(app, "selection_wire_composite.wgsl");
}

/// Spawns the mask camera, the line entity, and the mask render target.
pub(crate) fn spawn_selection_wire(app: &mut App, size: OutputSize) -> SelectionWire {
    let mask_target = add_selection_mask_target(app, size);
    let world = app.world_mut();
    let line_material = world
        .resource_mut::<Assets<SelectionWireMaterial>>()
        .add(SelectionWireMaterial::default());
    let line_mesh = {
        // MAIN_WORLD keeps the CPU vertex data writable after extraction, so
        // the system can re-upload lines when the selection changes.
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        write_line_mesh(&mut mesh, &[]);
        world.resource_mut::<Assets<Mesh>>().add(mesh)
    };
    let line_entity = world
        .spawn((
            SelectionWireLineEntity,
            Mesh3d(line_mesh.clone()),
            MeshMaterial3d(line_material.clone()),
            RenderLayers::layer(SELECTION_WIRE_LAYER),
            NoFrustumCulling,
            Transform::IDENTITY,
            Visibility::Hidden,
        ))
        .id();

    // The camera state is copied from the main camera by the sync system on
    // the first update, so defaults are fine for the initial spawn.
    let main_projection = Projection::default();
    let main_transform = Transform::IDENTITY;
    // The mask camera renders before the main camera so the composite node
    // always samples the current frame's mask.
    let mask_camera = world
        .spawn((
            SelectionMaskCamera,
            Camera3d::default(),
            CameraRenderGraph::new(SelectionWireMaskGraph),
            Tonemapping::None,
            DepthPrepass,
            RenderLayers::layer(SELECTION_WIRE_LAYER),
            RenderTarget::Image(mask_target.clone().into()),
            Camera {
                order: 0,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                is_active: true,
                ..Default::default()
            },
            main_projection,
            main_transform,
        ))
        .id();

    SelectionWire {
        mask_target,
        mask_camera,
        _line_entity: line_entity,
        line_mesh,
        _line_material: line_material,
    }
}

impl SelectionWire {
    /// Recreates the mask target after a viewport resize.
    pub(crate) fn resize(&mut self, app: &mut App, size: OutputSize) {
        let (removed, added) = {
            let world = app.world_mut();
            let removed = world
                .resource_mut::<Assets<Image>>()
                .remove(self.mask_target.id());
            let added = world
                .resource_mut::<Assets<Image>>()
                .add(new_selection_mask_image(size));
            (removed, added)
        };
        drop(removed);
        self.mask_target = added.clone();
        if let Some(mut target) = app.world_mut().get_mut::<RenderTarget>(self.mask_camera) {
            *target = RenderTarget::Image(added.into());
        }
    }
}

/// Copies the main camera state onto the mask camera so both views line up.
#[allow(clippy::type_complexity)]
fn sync_selection_mask_camera(
    main: Query<
        (&Projection, &GlobalTransform, &Camera),
        (
            With<MainPreviewCamera>,
            bevy::ecs::query::Without<SelectionMaskCamera>,
            bevy::ecs::query::Or<(
                bevy::ecs::query::Changed<Projection>,
                bevy::ecs::query::Changed<GlobalTransform>,
                bevy::ecs::query::Changed<Camera>,
            )>,
        ),
    >,
    mut mask: Query<
        (&mut Projection, &mut GlobalTransform, &mut Camera),
        (
            With<SelectionMaskCamera>,
            bevy::ecs::query::Without<MainPreviewCamera>,
        ),
    >,
) {
    let Ok((projection, transform, camera)) = main.single() else {
        return;
    };
    let Ok((mut mask_projection, mut mask_transform, mut mask_camera)) = mask.single_mut() else {
        return;
    };
    *mask_projection = projection.clone();
    *mask_transform = *transform;
    mask_camera.is_active = camera.is_active;
}

/// Rebuilds the wireframe line mesh when the selection or the camera changes.
fn update_selection_wire(
    selection: Res<SelectionGeometry>,
    main_camera: Query<bevy::ecs::change_detection::Ref<GlobalTransform>, With<MainPreviewCamera>>,
    wire: Res<SelectionWire>,
    mut line_visibility: Query<&mut Visibility, With<SelectionWireLineEntity>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut active: ResMut<SelectionWireActive>,
) {
    let Ok(camera_transform) = main_camera.single() else {
        return;
    };
    if !selection.is_changed() && !camera_transform.is_changed() {
        return;
    }
    let lines = selection_wire_lines(&selection, camera_transform.translation());
    if let Some(mut mesh) = meshes.get_mut(&wire.line_mesh) {
        write_line_mesh(&mut mesh, &lines);
    }
    let has_lines = !lines.is_empty();
    if let Ok(mut visibility) = line_visibility.single_mut() {
        *visibility = if has_lines {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    active.0 = has_lines;
}

/// Classifies the selected wireframe edges for the current camera position.
///
/// Interior edges stay silhouette-only: they are drawn only where front- and
/// back-facing faces meet, keeping internal mesh lines hidden. Boundary edges
/// follow their only face and are culled when it faces away from the camera.
fn selection_wire_lines(selection: &SelectionGeometry, camera_position: Vec3) -> Vec<(Vec3, Vec3)> {
    let selected_primitives = selection.selected_primitives();
    let selected_slot = selection.selected_slot();
    if selected_primitives.is_empty() && selected_slot.is_none() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for primitive in selection.primitives.iter().filter(|primitive| {
        selected_primitives.contains(&primitive.primitive_index)
            || selected_primitives.is_empty()
                && selected_slot.is_some_and(|slot_id| primitive.slot_id == slot_id)
    }) {
        for component in &primitive.components {
            for edge in &component.edges {
                let draw_edge = if edge.faces.len() == 1 {
                    component
                        .faces
                        .get(edge.faces[0])
                        .is_none_or(|face| faces_camera(face, camera_position))
                } else {
                    let mut has_front_face = false;
                    let mut has_back_face = false;
                    for &face_index in &edge.faces {
                        let Some(face) = component.faces.get(face_index) else {
                            continue;
                        };
                        if face.normal == Vec3::ZERO {
                            continue;
                        }
                        if faces_camera(face, camera_position) {
                            has_front_face = true;
                        } else {
                            has_back_face = true;
                        }
                    }
                    has_front_face && has_back_face
                };
                if draw_edge {
                    lines.push((edge.start, edge.end));
                }
            }
        }
    }
    lines
}

fn faces_camera(face: &SelectionFace, camera_position: Vec3) -> bool {
    // Degenerate faces have no orientation; keep their edges visible.
    face.normal == Vec3::ZERO || face.normal.dot(camera_position - face.center) > 0.0
}

/// Writes six quad vertices per line segment into the mesh.
fn write_line_mesh(mesh: &mut Mesh, lines: &[(Vec3, Vec3)]) {
    // Keep a degenerate sub-pixel line so the mesh always has vertices; the
    // line entity is hidden while the selection is empty.
    let fallback = [(Vec3::new(0.0, -1.0e3, 0.0), Vec3::new(1.0e-6, -1.0e3, 0.0))];
    let lines = if lines.is_empty() {
        &fallback[..]
    } else {
        lines
    };
    let mut positions_a = Vec::with_capacity(lines.len() * 6);
    let mut positions_b = Vec::with_capacity(lines.len() * 6);
    let mut corners = Vec::with_capacity(lines.len() * 6);
    for &(start, end) in lines {
        for corner in 0..6u32 {
            positions_a.push(start.to_array());
            positions_b.push(end.to_array());
            corners.push(corner);
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions_a);
    mesh.insert_attribute(SELECTION_WIRE_POSITION_B, positions_b);
    mesh.insert_attribute(SELECTION_WIRE_CORNER, corners);
}

fn new_selection_mask_image(size: OutputSize) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Linear storage: the line material writes straight color values and
        // the composite node reads them back without an sRGB round trip.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    image
}

fn add_selection_mask_target(app: &mut App, size: OutputSize) -> Handle<Image> {
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .add(new_selection_mask_image(size))
}

fn selection_wire_mask_schedule() -> Schedule {
    let mut schedule = Schedule::new(SelectionWireMaskGraph);
    schedule.set_build_settings(ScheduleBuildSettings {
        auto_insert_apply_deferred: false,
        ..Default::default()
    });
    // Depth prepass (selected object only) -> wireframe line quads (alpha
    // mask phase only, so transparent PMX materials never reach the mask) ->
    // write the mask to its image.
    schedule.add_systems((early_prepass, selection_wire_alpha_mask_pass, upscaling).chain());
    schedule
}

/// Renders only the alpha-mask phase into the mask color target.
///
/// The mask camera's depth buffer already holds the selected object's depth
/// from the prepass, so the line pipeline's hardware depth test hides line
/// fragments behind the object's own surface.
fn selection_wire_alpha_mask_pass(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
    )>,
    alpha_mask_phases: Res<ViewBinnedRenderPhases<AlphaMask3d>>,
    mut ctx: RenderContext,
) {
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth) = view.into_inner();
    let Some(alpha_mask_phase) = alpha_mask_phases.get(&extracted_view.retained_view_entity) else {
        return;
    };

    // Always begin the pass: the first color attachment use each frame clears
    // the mask, even when there is nothing to draw.
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("selection_wire_alpha_mask_pass"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = Viewport::from_viewport_and_override(camera.viewport.as_ref(), None) {
        render_pass.set_camera_viewport(&viewport);
    }
    if !alpha_mask_phase.is_empty()
        && let Err(err) = alpha_mask_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering the selection wire phase {err:?}");
    }
    drop(render_pass);
}

/// Pipeline for the fullscreen composite pass on the main camera.
#[derive(Resource)]
struct SelectionWireCompositePipeline {
    layout: BindGroupLayoutDescriptor,
    fragment_shader: Handle<Shader>,
    fullscreen_shader: FullscreenShader,
    sampler: Sampler,
}

fn init_selection_wire_composite_pipeline(
    render_device: Res<RenderDevice>,
    asset_server: Res<bevy::asset::AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    mut commands: Commands,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "selection_wire_composite_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("selection_wire_composite_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..Default::default()
    });
    commands.insert_resource(SelectionWireCompositePipeline {
        layout,
        fragment_shader: load_embedded_asset!(
            asset_server.as_ref(),
            "selection_wire_composite.wgsl"
        ),
        fullscreen_shader: fullscreen_shader.clone(),
        sampler,
    });
}

impl SpecializedRenderPipeline for SelectionWireCompositePipeline {
    type Key = TextureFormat;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("selection_wire_composite_pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.fragment_shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            ..default()
        }
    }
}

#[allow(clippy::type_complexity)]
fn prepare_view_selection_wire_composite_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<SelectionWireCompositePipeline>>,
    pipeline: Res<SelectionWireCompositePipeline>,
    views: Query<(Entity, &ExtractedView), (With<ViewTarget>, With<MainPreviewCamera>)>,
) {
    for (entity, view) in &views {
        let pipeline_id = pipelines.specialize(&pipeline_cache, &pipeline, view.target_format);
        commands
            .entity(entity)
            .insert(ViewSelectionWireCompositePipeline(pipeline_id));
    }
}

#[derive(Default)]
struct SelectionWireCompositeBindGroupCache(Option<(TextureViewId, TextureViewId, BindGroup)>);

/// Fullscreen composite node running on the main camera's Core3d schedule.
#[allow(clippy::too_many_arguments)]
fn selection_wire_composite(
    view: ViewQuery<(&ViewTarget, Option<&ViewSelectionWireCompositePipeline>)>,
    active: Res<SelectionWireActive>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<SelectionWireCompositePipeline>,
    mask_images: Res<RenderAssets<GpuImage>>,
    mask_handle: Res<SelectionWireMaskHandle>,
    mut cache: Local<SelectionWireCompositeBindGroupCache>,
    mut ctx: RenderContext,
) {
    if !active.0 {
        return;
    }
    let (target, view_pipeline) = view.into_inner();
    let Some(view_pipeline) = view_pipeline else {
        return;
    };
    let Some(render_pipeline) = pipeline_cache.get_render_pipeline(view_pipeline.0) else {
        return;
    };
    let Some(mask) = mask_images.get(&mask_handle.0) else {
        return;
    };

    let post_process = target.post_process_write();
    let source = post_process.source;
    let destination = post_process.destination;

    let bind_group = match cache.0.as_ref() {
        Some((source_id, mask_id, bind_group))
            if *source_id == source.id() && *mask_id == mask.texture_view.id() =>
        {
            bind_group.clone()
        }
        _ => {
            let bind_group = ctx.render_device().create_bind_group(
                "selection_wire_composite_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipeline.layout),
                &BindGroupEntries::sequential((source, &mask.texture_view, &pipeline.sampler)),
            );
            cache.0 = Some((source.id(), mask.texture_view.id(), bind_group.clone()));
            bind_group
        }
    };

    let pass_descriptor = RenderPassDescriptor {
        label: Some("selection_wire_composite"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: destination,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(Default::default()),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    };

    let mut render_pass = ctx.command_encoder().begin_render_pass(&pass_descriptor);
    render_pass.set_pipeline(render_pipeline);
    render_pass.set_bind_group(0, &bind_group, &[]);
    render_pass.draw(0..3, 0..1);
}
