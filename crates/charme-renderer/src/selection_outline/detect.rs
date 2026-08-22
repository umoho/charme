//! Fullscreen object-ID edge detection and compositing.
//!
//! Runs as a main-camera post-process node after tonemapping. For each pixel,
//! the four-way neighbours of the object-ID texture are compared; any pixel
//! whose neighbours hold a different ID is an object boundary and is painted
//! with the orange outline. Everything else passes the scene through
//! unchanged.

use bevy::{
    asset::Handle,
    asset::load_embedded_asset,
    core_pipeline::FullscreenShader,
    ecs::{component::Component, entity::Entity, query::With, resource::Resource, system::Query},
    prelude::{Commands, Local, Res, ResMut},
    render::{
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState,
            LoadOp, MultisampleState, Operations, PipelineCache, PrimitiveState,
            RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
            SamplerBindingType, SamplerDescriptor, ShaderStages, SpecializedRenderPipeline,
            SpecializedRenderPipelines, StoreOp, TextureFormat, TextureSampleType, TextureViewId,
            binding_types::{sampler, texture_2d},
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::GpuImage,
        view::{ExtractedView, ViewTarget},
    },
    shader::Shader,
    utils::default,
};

use super::{SelectionOutlineActive, SelectionOutlineIdHandle};
use crate::backend::MainPreviewCamera;

/// Registers the embedded detect shader. The asset path is resolved relative
/// to this module, so it must match the `load_embedded_asset!` counterpart.
pub(crate) fn install_detect_shader(app: &mut bevy::app::App) {
    bevy::asset::embedded_asset!(app, "selection_outline_detect.wgsl");
}

/// Pipeline for the fullscreen edge-detection composite node.
#[derive(Resource)]
pub(crate) struct SelectionOutlineDetectPipeline {
    layout: BindGroupLayoutDescriptor,
    fragment_shader: Handle<Shader>,
    fullscreen_shader: FullscreenShader,
    sampler: Sampler,
}

/// Marker for a view's detect pipeline.
#[derive(Component)]
pub(crate) struct ViewSelectionOutlineDetectPipeline(CachedRenderPipelineId);

pub(crate) fn init_selection_outline_detect_pipeline(
    render_device: Res<RenderDevice>,
    asset_server: Res<bevy::asset::AssetServer>,
    fullscreen_shader: Res<FullscreenShader>,
    mut commands: Commands,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "selection_outline_detect_layout",
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
        label: Some("selection_outline_detect_sampler"),
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..Default::default()
    });
    commands.insert_resource(SelectionOutlineDetectPipeline {
        layout,
        fragment_shader: load_embedded_asset!(
            asset_server.as_ref(),
            "selection_outline_detect.wgsl"
        ),
        fullscreen_shader: fullscreen_shader.clone(),
        sampler,
    });
}

impl SpecializedRenderPipeline for SelectionOutlineDetectPipeline {
    type Key = TextureFormat;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("selection_outline_detect_pipeline".into()),
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
pub(crate) fn prepare_view_selection_outline_detect_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<SelectionOutlineDetectPipeline>>,
    pipeline: Res<SelectionOutlineDetectPipeline>,
    views: Query<(Entity, &ExtractedView), (With<ViewTarget>, With<MainPreviewCamera>)>,
) {
    for (entity, view) in &views {
        let pipeline_id = pipelines.specialize(&pipeline_cache, &pipeline, view.target_format);
        commands
            .entity(entity)
            .insert(ViewSelectionOutlineDetectPipeline(pipeline_id));
    }
}

#[derive(Default)]
pub(crate) struct SelectionOutlineDetectBindGroupCache(
    Option<(TextureViewId, TextureViewId, BindGroup)>,
);

/// Fullscreen edge-detection node running on the main camera's Core3d schedule.
#[allow(clippy::too_many_arguments)]
pub(crate) fn selection_outline_detect(
    view: ViewQuery<(&ViewTarget, Option<&ViewSelectionOutlineDetectPipeline>)>,
    active: Res<SelectionOutlineActive>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<SelectionOutlineDetectPipeline>,
    id_images: Res<RenderAssets<GpuImage>>,
    id_handle: Res<SelectionOutlineIdHandle>,
    mut cache: Local<SelectionOutlineDetectBindGroupCache>,
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
    let Some(id) = id_images.get(&id_handle.0) else {
        return;
    };

    let post_process = target.post_process_write();
    let source = post_process.source;
    let destination = post_process.destination;

    let bind_group = match cache.0.as_ref() {
        Some((source_id, id_id, bind_group))
            if *source_id == source.id() && *id_id == id.texture_view.id() =>
        {
            bind_group.clone()
        }
        _ => {
            let bind_group = ctx.render_device().create_bind_group(
                "selection_outline_detect_bind_group",
                &pipeline_cache.get_bind_group_layout(&pipeline.layout),
                &BindGroupEntries::sequential((source, &id.texture_view, &pipeline.sampler)),
            );
            cache.0 = Some((source.id(), id.texture_view.id(), bind_group.clone()));
            bind_group
        }
    };

    let mut render_pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("selection_outline_detect"),
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
        });
    render_pass.set_pipeline(render_pipeline);
    render_pass.set_bind_group(0, &bind_group, &[]);
    render_pass.draw(0..3, 0..1);
}
