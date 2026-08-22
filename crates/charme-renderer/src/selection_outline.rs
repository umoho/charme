//! Selected-object outline rendered through object-ID edge detection.
//!
//! This mirrors Blender's `Outline Selected` overlay (Viewport Overlays >
//! Objects > Outline Selected): selected objects are rendered into a
//! dedicated object-ID texture, and a fullscreen pass compares neighbouring
//! IDs to detect object boundaries. The implementation is split in two
//! modules:
//!
//! * [`id`] renders the object-ID map into an offscreen texture using a
//!   dedicated camera and a per-primitive ID material.
//! * [`detect`] runs a fullscreen pass on the main camera that finds ID
//!   discontinuities and composites the orange outline over the final frame.
//!
//! See `source/blender/draw/engines/overlay/overlay_outline.hh` and
//! `source/blender/draw/engines/overlay/shaders/overlay_outline_detect_frag.glsl`
//! for the Blender reference implementation.

mod detect;
mod id;

use bevy::{
    app::{App, Plugin, PostUpdate},
    asset::Handle,
    ecs::{resource::Resource, schedule::IntoScheduleConfigs},
    image::Image,
    render::{
        GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::ExtractComponentPlugin,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::SpecializedRenderPipelines,
    },
    transform::TransformSystems,
};

pub(crate) use id::{
    OutlineIdMaterial, SelectionIdCamera, SelectionOutline, bake_outline_id,
    spawn_selection_outline,
};

use crate::backend::MainPreviewCamera;

/// Render layer of the selection ID camera and its per-primitive ID proxies.
pub(crate) const SELECTION_ID_LAYER: usize = 1;

/// Whether the composite pass has an outline to draw this frame.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionOutlineActive(pub(crate) bool);

/// The main-world handle of the ID texture, extracted for the composite node.
#[derive(Resource, ExtractResource, Clone, Debug, Default)]
pub(crate) struct SelectionOutlineIdHandle(pub(crate) Handle<Image>);

/// Installs the two outline modules and their render systems.
///
/// Must be called before `App::finish` so the render app picks up the
/// composite node. The ID camera itself is spawned later by the backend
/// through [`spawn_selection_outline`].
pub(crate) struct SelectionOutlinePlugin;

impl Plugin for SelectionOutlinePlugin {
    fn build(&self, app: &mut App) {
        id::install_outline_id_material(app);
        detect::install_detect_shader(app);

        app.add_plugins((
            ExtractResourcePlugin::<SelectionOutlineActive>::default(),
            ExtractResourcePlugin::<SelectionOutlineIdHandle>::default(),
            ExtractComponentPlugin::<MainPreviewCamera>::default(),
            ExtractComponentPlugin::<SelectionIdCamera>::default(),
        ))
        .init_resource::<SelectionOutlineActive>()
        .init_resource::<SelectionOutlineIdHandle>()
        .init_resource::<SelectionOutline>()
        .add_systems(
            PostUpdate,
            id::sync_selection_id_camera.after(TransformSystems::Propagate),
        );

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_gpu_resource::<SpecializedRenderPipelines<
                    detect::SelectionOutlineDetectPipeline,
                >>()
                .add_systems(
                    RenderStartup,
                    detect::init_selection_outline_detect_pipeline,
                )
                .add_systems(
                    bevy::core_pipeline::schedule::Core3d,
                    detect::selection_outline_detect
                        .in_set(bevy::core_pipeline::schedule::Core3dSystems::PostProcess)
                        .after(bevy::core_pipeline::tonemapping::tonemapping),
                )
                .add_systems(
                    Render,
                    detect::prepare_view_selection_outline_detect_pipelines
                        .in_set(RenderSystems::Prepare),
                );
        }
    }
}
