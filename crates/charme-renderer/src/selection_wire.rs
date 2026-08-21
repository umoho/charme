//! Selection wireframe rendering.
//!
//! The wireframe mimics Blender's selected-object wire overlay:
//!
//! * Interior front-side mesh lines stay hidden, while back-side edges
//!   participate in the wireframe (the CPU-side edge classification in
//!   [`update_selection_wire`]).
//! * The wireframe is never occluded by other objects, but is occluded by the
//!   selected object itself.
//!
//! This is achieved with a dedicated offscreen "mask" camera:
//!
//! * The mask camera runs a custom render schedule containing only the depth
//!   prepass (writing the selected object's depth into the mask view depth
//!   buffer), a line pass that draws the wireframe quads directly, and the
//!   upscaling blit that writes the mask image.
//! * Because no other geometry renders into the mask camera, the hardware
//!   depth test of the line pipeline provides self-occlusion for free while
//!   other objects can never occlude the wireframe.
//! * A fullscreen composite node on the main camera blends the mask image
//!   over the final frame.

use bevy::{
    app::{App, PostUpdate},
    asset::{Handle, RenderAssetUsages, load_embedded_asset},
    camera::{
        Camera, Camera3d, ClearColorConfig, Projection, RenderTarget, Viewport,
        visibility::RenderLayers,
    },
    core_pipeline::{
        FullscreenShader,
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
    math::Vec3,
    prelude::{
        Assets, Color, Commands, Component, Entity, GlobalTransform, Local, Query, Res, ResMut,
        Resource, Transform,
    },
    render::{
        GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems,
        camera::{CameraRenderGraph, ExtractedCamera},
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
            BufferId, BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState,
            ColorWrites, CompareFunction, DepthStencilState, Extent3d, FilterMode, FragmentState,
            LoadOp, MultisampleState, Operations, PipelineCache, PrimitiveState,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
            SamplerBindingType, SamplerDescriptor, ShaderStages, SpecializedRenderPipeline,
            SpecializedRenderPipelines, StoreOp, TextureDimension, TextureFormat,
            TextureSampleType, TextureUsages, TextureViewId, VertexAttribute, VertexFormat,
            VertexStepMode,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::GpuImage,
        view::{
            ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset,
            ViewUniforms,
        },
    },
    shader::Shader,
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
#[derive(Component, ExtractComponent, Clone)]
struct SelectionMaskCamera;

/// Marker for the main camera's composite pipeline.
#[derive(Component)]
struct ViewSelectionWireCompositePipeline(CachedRenderPipelineId);

/// Marker for a view's line pipeline.
#[derive(Component)]
struct ViewSelectionWireLinePipeline(CachedRenderPipelineId);

/// Whether the composite pass has wireframe lines to draw this frame.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionWireActive(bool);

/// The main-world handle of the wireframe mask image, extracted for the
/// composite node.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionWireMaskHandle(pub(crate) Handle<Image>);

/// The classified wireframe line segments, extracted for the line pass.
#[derive(Resource, ExtractResource, Clone, Debug, Default, PartialEq)]
pub(crate) struct SelectionWireLines {
    lines: Vec<(Vec3, Vec3)>,
}

/// Main-world handles describing the selection wireframe setup.
#[derive(Resource, Debug, Clone)]
pub(crate) struct SelectionWire {
    pub(crate) mask_target: Handle<Image>,
    pub(crate) mask_camera: Entity,
}

impl Default for SelectionWire {
    fn default() -> Self {
        Self {
            mask_target: Handle::default(),
            mask_camera: Entity::PLACEHOLDER,
        }
    }
}

/// Schedule label for the selection mask camera.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct SelectionWireMaskGraph;

const LINE_VERTEX_SIZE: u64 = 28; // vec3 + vec3 + u32
const LINE_CORNERS: [u32; 6] = [0, 1, 2, 3, 4, 5];

/// Registers the selection wireframe systems, resources, and render schedule.
///
/// Must be called before `App::finish` so the render app picks up the mask
/// schedule and the composite node.
pub(crate) fn install_selection_wire(app: &mut App) {
    app.add_plugins((
        ExtractResourcePlugin::<SelectionWireActive>::default(),
        ExtractResourcePlugin::<SelectionWireMaskHandle>::default(),
        ExtractResourcePlugin::<SelectionWireLines>::default(),
        ExtractComponentPlugin::<MainPreviewCamera>::default(),
        ExtractComponentPlugin::<SelectionMaskCamera>::default(),
    ))
    .init_resource::<SelectionWireActive>()
    .init_resource::<SelectionWireMaskHandle>()
    .init_resource::<SelectionWireLines>()
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
            .init_gpu_resource::<SpecializedRenderPipelines<SelectionWireLinePipeline>>()
            .init_resource::<SelectionWireGpuBuffer>()
            .add_systems(
                RenderStartup,
                (
                    init_selection_wire_composite_pipeline,
                    init_selection_wire_line_pipeline,
                ),
            )
            .add_systems(
                Core3d,
                selection_wire_composite
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
            )
            .add_systems(
                Render,
                (
                    prepare_view_selection_wire_composite_pipelines,
                    prepare_view_selection_wire_line_pipelines,
                    prepare_selection_wire_gpu_buffer,
                )
                    .in_set(RenderSystems::Prepare),
            );
    }

    bevy::asset::embedded_asset!(app, "selection_wire_lines.wgsl");
    bevy::asset::embedded_asset!(app, "selection_wire_composite.wgsl");
}

/// Spawns the mask camera and the mask render target.
pub(crate) fn spawn_selection_wire(app: &mut App, size: OutputSize) -> SelectionWire {
    let mask_target = add_selection_mask_target(app, size);
    let world = app.world_mut();

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

/// Reclassifies the selected wireframe edges when the selection or the camera
/// changes.
fn update_selection_wire(
    selection: Res<SelectionGeometry>,
    main_camera: Query<bevy::ecs::change_detection::Ref<GlobalTransform>, With<MainPreviewCamera>>,
    mut lines_resource: ResMut<SelectionWireLines>,
    mut active: ResMut<SelectionWireActive>,
) {
    let Ok(camera_transform) = main_camera.single() else {
        return;
    };
    if !selection.is_changed() && !camera_transform.is_changed() {
        return;
    }
    let lines = selection_wire_lines(&selection, camera_transform.translation());
    active.0 = !lines.is_empty();
    lines_resource.lines = lines;
}

/// Classifies the selected wireframe edges for the current camera position.
///
/// Interior edges stay hidden only when every adjacent face faces the camera
/// (internal front-side mesh lines). Any edge touching a back-facing face is
/// part of the wireframe, and boundary edges always are: the GPU depth test
/// against the object's own surface then hides the fragments that are
/// actually occluded.
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
                    true
                } else {
                    let mut has_back_face = false;
                    for &face_index in &edge.faces {
                        let Some(face) = component.faces.get(face_index) else {
                            continue;
                        };
                        if face.normal == Vec3::ZERO {
                            continue;
                        }
                        if !faces_camera(face, camera_position) {
                            has_back_face = true;
                            break;
                        }
                    }
                    has_back_face
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

fn new_selection_mask_image(size: OutputSize) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Linear storage: the line pass writes straight color values and the
        // composite node reads them back without an sRGB round trip.
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
    // Depth prepass (selected object only) -> wireframe line quads (drawn
    // directly; no other material reaches the mask color target) -> write the
    // mask to its image.
    schedule.add_systems((early_prepass, selection_wire_line_pass, upscaling).chain());
    schedule
}

/// GPU buffer holding the expanded line quad vertices.
#[derive(Resource, Default)]
struct SelectionWireGpuBuffer {
    buffer: Option<Buffer>,
    vertex_count: u32,
    cached_lines: Vec<(Vec3, Vec3)>,
}

/// Rebuilds the line vertex buffer when the classified lines change.
fn prepare_selection_wire_gpu_buffer(
    lines: Res<SelectionWireLines>,
    render_device: Res<RenderDevice>,
    mut gpu: ResMut<SelectionWireGpuBuffer>,
) {
    if lines.lines == gpu.cached_lines {
        return;
    }
    gpu.cached_lines = lines.lines.clone();
    if lines.lines.is_empty() {
        gpu.buffer = None;
        gpu.vertex_count = 0;
        return;
    }

    let mut data = Vec::with_capacity(lines.lines.len() * 6 * LINE_VERTEX_SIZE as usize);
    for &(start, end) in &lines.lines {
        for corner in LINE_CORNERS {
            data.extend_from_slice(&start.to_array().map(f32::to_le_bytes).concat());
            data.extend_from_slice(&end.to_array().map(f32::to_le_bytes).concat());
            data.extend_from_slice(&corner.to_le_bytes());
        }
    }
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("selection_wire_lines"),
        contents: &data,
        usage: BufferUsages::VERTEX,
    });
    gpu.vertex_count = (lines.lines.len() * 6) as u32;
    gpu.buffer = Some(buffer);
}

/// Pipeline drawing the wireframe line quads.
#[derive(Resource)]
struct SelectionWireLinePipeline {
    view_layout: BindGroupLayoutDescriptor,
    vertex_shader: Handle<Shader>,
    fragment_shader: Handle<Shader>,
}

fn init_selection_wire_line_pipeline(
    asset_server: Res<bevy::asset::AssetServer>,
    mut commands: Commands,
) {
    commands.insert_resource(SelectionWireLinePipeline {
        view_layout: BindGroupLayoutDescriptor::new(
            "selection_wire_line_view_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (uniform_buffer::<ViewUniform>(true),),
            ),
        ),
        vertex_shader: load_embedded_asset!(asset_server.as_ref(), "selection_wire_lines.wgsl"),
        fragment_shader: load_embedded_asset!(asset_server.as_ref(), "selection_wire_lines.wgsl"),
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SelectionWireLinePipelineKey {
    format: TextureFormat,
    msaa_samples: u32,
}

impl SpecializedRenderPipeline for SelectionWireLinePipeline {
    type Key = SelectionWireLinePipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("selection_wire_line_pipeline".into()),
            layout: vec![self.view_layout.clone()],
            immediate_size: 0,
            vertex: bevy::render::render_resource::VertexState {
                shader: self.vertex_shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("vertex".into()),
                buffers: vec![bevy::mesh::VertexBufferLayout {
                    array_stride: LINE_VERTEX_SIZE,
                    step_mode: VertexStepMode::Vertex,
                    attributes: vec![
                        VertexAttribute {
                            format: VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        VertexAttribute {
                            format: VertexFormat::Uint32,
                            offset: 24,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            fragment: Some(FragmentState {
                shader: self.fragment_shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: MultisampleState {
                count: key.msaa_samples,
                ..Default::default()
            },
            zero_initialize_workgroup_memory: true,
        }
    }
}

#[allow(clippy::type_complexity)]
fn prepare_view_selection_wire_line_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<SelectionWireLinePipeline>>,
    pipeline: Res<SelectionWireLinePipeline>,
    views: Query<(Entity, &ExtractedView, &Msaa), (With<ViewTarget>, With<SelectionMaskCamera>)>,
) {
    for (entity, view, msaa) in &views {
        let pipeline_id = pipelines.specialize(
            &pipeline_cache,
            &pipeline,
            SelectionWireLinePipelineKey {
                format: view.target_format,
                msaa_samples: msaa.samples(),
            },
        );
        commands
            .entity(entity)
            .insert(ViewSelectionWireLinePipeline(pipeline_id));
    }
}

#[derive(Default)]
struct SelectionWireLineBindGroupCache(Option<(BufferId, u32, BindGroup)>);

/// Renders the wireframe line quads into the mask color target.
///
/// The mask camera's depth buffer already holds the selected object's depth
/// from the prepass, so the hardware depth test hides line fragments behind
/// the object's own surface.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn selection_wire_line_pass(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
        &ViewUniformOffset,
        Option<&ViewSelectionWireLinePipeline>,
    )>,
    pipeline_cache: Res<PipelineCache>,
    gpu: Res<SelectionWireGpuBuffer>,
    view_uniforms: Res<ViewUniforms>,
    mut cache: Local<SelectionWireLineBindGroupCache>,
    mut ctx: RenderContext,
) {
    let (camera, _extracted_view, target, depth, view_uniform_offset, view_pipeline) =
        view.into_inner();
    let Some(view_pipeline) = view_pipeline else {
        return;
    };
    let render_pipeline = pipeline_cache.get_render_pipeline(view_pipeline.0);

    // Prepare the bind group before starting the pass so `ctx` stays free.
    let bind_group = if render_pipeline.is_some() && gpu.buffer.is_some() {
        let view_uniforms_buffer = view_uniforms.uniforms.buffer().unwrap();
        match &cache.0 {
            Some((buffer_id, offset, bind_group))
                if *buffer_id == view_uniforms_buffer.id()
                    && *offset == view_uniform_offset.offset =>
            {
                Some(bind_group.clone())
            }
            _ => {
                let bind_group = ctx.render_device().create_bind_group(
                    "selection_wire_line_view_bind_group",
                    &pipeline_cache.get_bind_group_layout(
                        &world.resource::<SelectionWireLinePipeline>().view_layout,
                    ),
                    &BindGroupEntries::single(&view_uniforms.uniforms),
                );
                cache.0 = Some((
                    view_uniforms_buffer.id(),
                    view_uniform_offset.offset,
                    bind_group.clone(),
                ));
                Some(bind_group)
            }
        }
    } else {
        None
    };

    // Always begin the pass: the first color attachment use each frame clears
    // the mask, even when there is nothing to draw.
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("selection_wire_line_pass"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = Viewport::from_viewport_and_override(camera.viewport.as_ref(), None) {
        render_pass.set_camera_viewport(&viewport);
    }

    if let (Some(buffer), Some(render_pipeline), Some(bind_group)) =
        (gpu.buffer.as_ref(), render_pipeline, bind_group.as_ref())
    {
        render_pass.set_render_pipeline(render_pipeline);
        render_pass.set_bind_group(0, bind_group, &[view_uniform_offset.offset]);
        render_pass.set_vertex_buffer(0, buffer.slice(..));
        render_pass.draw(0..gpu.vertex_count, 0..1);
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
