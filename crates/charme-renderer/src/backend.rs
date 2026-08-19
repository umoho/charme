use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread,
    time::Duration,
};

use bevy::{
    DefaultPlugins,
    app::{App, PluginGroup, PluginsState, PostUpdate},
    asset::{Assets, RenderAssetUsages},
    camera::{
        Camera, Camera3d, ClearColorConfig, Projection, RenderTarget, visibility::RenderLayers,
    },
    core_pipeline::tonemapping::Tonemapping,
    ecs::schedule::IntoScheduleConfigs,
    gizmos::prelude::{DefaultGizmoConfigGroup, GizmoConfigStore, Gizmos},
    image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::VertexAttributeValues,
    prelude::{
        Color, Commands, Component, Cuboid, DirectionalLight, Entity, GlobalTransform, Mesh,
        Mesh3d, MeshBuilder, MeshMaterial3d, Meshable, On, Plane3d, Quat, Res, Sphere,
        StandardMaterial, Transform, Vec2, Vec3,
    },
    render::{
        RenderApp, RenderPlugin,
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
    transform::TransformSystems,
};

use charme_bevy::{CharmeMaterial, CharmeMaterialPlugin, ParameterError};
use charme_core::{MaterialSlotId, ParameterValue};

use crate::{
    BackgroundColor, Frame, OutputSize, PixelFormat, PmxLoadProgress, PmxLoadRequest, PmxLoadStage,
    PmxSourceIdentity, RendererConfig, RendererError,
    renderer::{RendererNotification, ViewportSelectionAction},
    scene::{
        PreparedPmxScene, SelectionGeometry, SpawnedPmxScene, prepare_pmx_scene, spawn_pmx_scene,
    },
};

pub(crate) enum Command {
    Resize(OutputSize),
    SetBackground(BackgroundColor),
    Orbit {
        delta_x: f32,
        delta_y: f32,
    },
    Zoom(f32),
    ResetCamera,
    LoadPmx(PmxLoadRequest),
    ClearPmx,
    SetMaterialParameter {
        slot_id: Option<MaterialSlotId>,
        path: String,
        value: ParameterValue,
    },
    SetSelectedMaterialSlot(Option<MaterialSlotId>),
    SetSelectedPrimitives(Vec<usize>),
    PickViewport {
        x: f32,
        y: f32,
        selection_action: ViewportSelectionAction,
    },
    RequestMaterialInspectorPreview {
        slot_id: Option<MaterialSlotId>,
        slot_index: Option<usize>,
    },
    Redraw,
    Shutdown,
}

pub(crate) enum WorkerEvent {
    Frame(Frame),
    Notification(RendererNotification),
    Error(RendererError),
}

enum LoadTaskEvent {
    Progress(PmxLoadProgress),
    Prepared {
        request_id: u64,
        prepared: Box<PreparedPmxScene>,
    },
    Failed {
        request_id: u64,
        source: PmxSourceIdentity,
        message: String,
    },
}

enum Completion {
    Frame,
    MaterialThumbnail { slot_index: usize },
    MaterialInspectorPreview { slot_index: usize },
}

pub(crate) fn spawn(
    config: RendererConfig,
    commands: Receiver<Command>,
    events: Sender<WorkerEvent>,
    initialized: SyncSender<Result<(), RendererError>>,
) -> Result<thread::JoinHandle<()>, RendererError> {
    thread::Builder::new()
        .name("charme-renderer".to_owned())
        .spawn(move || {
            let initialization_reporter = initialized.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                run(config, commands, events.clone(), initialized)
            }));

            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let _ = initialization_reporter.try_send(Err(error.clone()));
                    let _ = events.send(WorkerEvent::Error(error));
                }
                Err(_) => {
                    let _ = initialization_reporter.try_send(Err(
                        RendererError::InitializationFailed {
                            message: "the rendering worker terminated during startup".to_owned(),
                        },
                    ));
                    let _ = events.send(WorkerEvent::Error(RendererError::WorkerPanicked));
                }
            }
        })
        .map_err(|error| RendererError::InitializationFailed {
            message: format!("could not start the rendering worker: {error}"),
        })
}

fn run(
    config: RendererConfig,
    commands: Receiver<Command>,
    events: Sender<WorkerEvent>,
    initialized: SyncSender<Result<(), RendererError>>,
) -> Result<(), RendererError> {
    let (completion_tx, completion_rx) = mpsc::channel();
    let (load_tx, load_rx) = mpsc::channel();
    let mut backend = Backend::new(&config, events, completion_tx, load_tx, load_rx)?;
    initialized
        .send(Ok(()))
        .map_err(|_| RendererError::WorkerStopped)?;

    let mut dirty = false;
    let mut in_flight = false;

    loop {
        while let Ok(completion) = completion_rx.try_recv() {
            match completion {
                Completion::Frame => in_flight = false,
                Completion::MaterialThumbnail { slot_index } => {
                    backend.finish_material_thumbnail(slot_index);
                }
                Completion::MaterialInspectorPreview { slot_index } => {
                    backend.finish_material_inspector_preview(slot_index);
                }
            }
        }

        let command = if in_flight
            || dirty
            || backend.pending_thumbnails != 0
            || backend.pending_inspector_preview
            || backend.pending_load_tasks != 0
            || backend.requested_inspector_slot.is_some()
            || !backend.thumbnail_queue.is_empty()
        {
            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return Ok(()),
            }
        };

        if let Some(command) = command {
            match command {
                Command::Resize(size) => {
                    backend.resize(size)?;
                    dirty = true;
                }
                Command::SetBackground(background) => {
                    backend.set_background(background)?;
                    dirty = true;
                }
                Command::Orbit { delta_x, delta_y } => {
                    backend.orbit(delta_x, delta_y)?;
                    dirty = true;
                }
                Command::Zoom(delta) => {
                    backend.zoom(delta)?;
                    dirty = true;
                }
                Command::ResetCamera => {
                    backend.reset_camera()?;
                    dirty = true;
                }
                Command::LoadPmx(request) => {
                    dirty |= backend.load_pmx(request)?;
                }
                Command::ClearPmx => {
                    backend.clear_pmx()?;
                    dirty = true;
                }
                Command::SetMaterialParameter {
                    slot_id,
                    path,
                    value,
                } => {
                    dirty |= backend.set_material_parameter(slot_id, path, value);
                }
                Command::SetSelectedMaterialSlot(slot_id) => {
                    dirty |= backend.set_selected_material_slot(slot_id);
                }
                Command::SetSelectedPrimitives(primitive_indices) => {
                    dirty |= backend.set_selected_primitives(primitive_indices);
                }
                Command::PickViewport {
                    x,
                    y,
                    selection_action,
                } => {
                    backend.pick_viewport(x, y, selection_action)?;
                }
                Command::Redraw => dirty = true,
                Command::RequestMaterialInspectorPreview {
                    slot_id,
                    slot_index,
                } => {
                    backend.request_material_inspector_preview(slot_id, slot_index);
                }
                Command::Shutdown => {
                    backend.join_load_tasks();
                    return Ok(());
                }
            }
        }

        loop {
            match commands.try_recv() {
                Ok(Command::Resize(size)) => {
                    backend.resize(size)?;
                    dirty = true;
                }
                Ok(Command::SetBackground(background)) => {
                    backend.set_background(background)?;
                    dirty = true;
                }
                Ok(Command::Orbit { delta_x, delta_y }) => {
                    backend.orbit(delta_x, delta_y)?;
                    dirty = true;
                }
                Ok(Command::Zoom(delta)) => {
                    backend.zoom(delta)?;
                    dirty = true;
                }
                Ok(Command::ResetCamera) => {
                    backend.reset_camera()?;
                    dirty = true;
                }
                Ok(Command::LoadPmx(request)) => {
                    dirty |= backend.load_pmx(request)?;
                }
                Ok(Command::ClearPmx) => {
                    backend.clear_pmx()?;
                    dirty = true;
                }
                Ok(Command::SetMaterialParameter {
                    slot_id,
                    path,
                    value,
                }) => {
                    dirty |= backend.set_material_parameter(slot_id, path, value);
                }
                Ok(Command::SetSelectedMaterialSlot(slot_id)) => {
                    dirty |= backend.set_selected_material_slot(slot_id);
                }
                Ok(Command::SetSelectedPrimitives(primitive_indices)) => {
                    dirty |= backend.set_selected_primitives(primitive_indices);
                }
                Ok(Command::PickViewport {
                    x,
                    y,
                    selection_action,
                }) => {
                    backend.pick_viewport(x, y, selection_action)?;
                }
                Ok(Command::Redraw) => dirty = true,
                Ok(Command::RequestMaterialInspectorPreview {
                    slot_id,
                    slot_index,
                }) => {
                    backend.request_material_inspector_preview(slot_id, slot_index);
                }
                Ok(Command::Shutdown) => {
                    backend.join_load_tasks();
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        dirty |= backend.poll_load_events()?;

        if dirty && backend.size.is_empty() {
            dirty = false;
        } else if dirty && !in_flight {
            backend.request_readback();
            dirty = false;
            in_flight = true;
        }

        // Keep viewport frames responsive while thumbnails are generated in
        // the background. Only start the next thumbnail when the main camera
        // has no pending redraw or readback.
        if !dirty && !in_flight {
            backend.start_next_thumbnail_readback();
            if backend.pending_thumbnails == 0 {
                backend.start_inspector_preview_readback();
            }
        }

        if in_flight || backend.pending_thumbnails != 0 || backend.pending_inspector_preview {
            backend.app.update();
        }
    }
}

struct Backend {
    app: App,
    size: OutputSize,
    pixel_format: PixelFormat,
    target: bevy::prelude::Handle<Image>,
    camera: Entity,
    placeholder_entities: Vec<Entity>,
    placeholder_materials: Vec<bevy::prelude::Handle<CharmeMaterial>>,
    pmx_scene: Option<SpawnedPmxScene>,
    scene_request_id: Option<u64>,
    orbit: OrbitState,
    initial_orbit: OrbitState,
    next_sequence: u64,
    events: Sender<WorkerEvent>,
    completion: Sender<Completion>,
    load_events: Receiver<LoadTaskEvent>,
    load_event_sender: Sender<LoadTaskEvent>,
    load_workers: Vec<thread::JoinHandle<()>>,
    pending_load_tasks: usize,
    latest_load_request_id: Option<u64>,
    next_load_request_id: u64,
    pending_thumbnails: usize,
    pending_inspector_preview: bool,
    requested_inspector_slot: Option<MaterialSlotId>,
    thumbnail_preview: MaterialPreviewStudio,
    inspector_preview: MaterialPreviewStudio,
    material_previews: Vec<MaterialPreview>,
    thumbnail_queue: VecDeque<usize>,
}

struct MaterialPreviewStudio {
    target: bevy::prelude::Handle<Image>,
    object: Entity,
    camera: Entity,
    _floor: Option<Entity>,
    _lights: [Entity; 2],
    fallback_material: bevy::prelude::Handle<CharmeMaterial>,
}

struct MaterialPreview {
    request_id: u64,
    slot_id: MaterialSlotId,
    slot_index: usize,
    source: PmxSourceIdentity,
    material: bevy::prelude::Handle<CharmeMaterial>,
}

const MATERIAL_PREVIEW_SIZE: u32 = 64;
const MATERIAL_INSPECTOR_PREVIEW_SIZE: u32 = 256;
const MATERIAL_THUMBNAIL_LAYER: usize = 30;
const MATERIAL_INSPECTOR_LAYER: usize = 31;

impl Backend {
    fn new(
        config: &RendererConfig,
        events: Sender<WorkerEvent>,
        completion: Sender<Completion>,
        load_event_sender: Sender<LoadTaskEvent>,
        load_events: Receiver<LoadTaskEvent>,
    ) -> Result<Self, RendererError> {
        let mut app = App::new();
        // This renderer already owns a private worker thread and advances the
        // Bevy app synchronously. Keep Bevy's `multi_threaded` feature off so
        // its task pools and render pipeline do not add another thread layer.
        app.add_plugins(
            DefaultPlugins
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..Default::default()
                })
                .build(),
        );
        app.add_plugins(CharmeMaterialPlugin);
        app.init_resource::<SelectionGeometry>().add_systems(
            PostUpdate,
            draw_selected_primitive_gizmo.after(TransformSystems::Propagate),
        );

        while app.plugins_state() == PluginsState::Adding {
            thread::yield_now();
        }
        app.finish();
        app.cleanup();

        if let Some(mut configs) = app.world_mut().get_resource_mut::<GizmoConfigStore>() {
            let (config, _) = configs.config_mut::<DefaultGizmoConfigGroup>();
            config.depth_bias = -0.1;
            config.line.width = 2.5;
        }

        if app.get_sub_app(RenderApp).is_none() {
            return Err(RendererError::DeviceUnavailable);
        }

        let texture_size = usable_texture_size(config.output_size);
        let target = add_target_image(&mut app, texture_size, config.pixel_format);
        let thumbnail_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Sphere::new(1.0).mesh().ico(3).unwrap());
        let orbit = OrbitState::default();
        let (camera, placeholder_entities, placeholder_materials) = spawn_scene(
            &mut app,
            target.clone(),
            config.background,
            !config.output_size.is_empty(),
            orbit,
        );

        let thumbnail_preview = spawn_material_preview_studio(
            &mut app,
            thumbnail_mesh.clone(),
            MATERIAL_PREVIEW_SIZE,
            MATERIAL_THUMBNAIL_LAYER,
            false,
        );
        let inspector_preview = spawn_material_preview_studio(
            &mut app,
            thumbnail_mesh,
            MATERIAL_INSPECTOR_PREVIEW_SIZE,
            MATERIAL_INSPECTOR_LAYER,
            true,
        );

        // Make the image and camera visible to the render world before reporting
        // successful initialization.
        app.update();

        Ok(Self {
            app,
            size: config.output_size,
            pixel_format: config.pixel_format,
            target,
            camera,
            placeholder_entities,
            placeholder_materials,
            pmx_scene: None,
            scene_request_id: None,
            orbit,
            initial_orbit: orbit,
            next_sequence: 1,
            events,
            completion,
            load_events,
            load_event_sender,
            load_workers: Vec::new(),
            pending_load_tasks: 0,
            latest_load_request_id: None,
            next_load_request_id: 1,
            pending_thumbnails: 0,
            pending_inspector_preview: false,
            requested_inspector_slot: None,
            thumbnail_preview,
            inspector_preview,
            material_previews: Vec::new(),
            thumbnail_queue: VecDeque::new(),
        })
    }

    fn resize(&mut self, size: OutputSize) -> Result<(), RendererError> {
        if size == self.size {
            return Ok(());
        }

        self.size = size;
        if size.is_empty() {
            let mut camera = self
                .app
                .world_mut()
                .get_mut::<Camera>(self.camera)
                .ok_or_else(|| RendererError::RenderingFailed {
                    message: "the internal camera is unavailable".to_owned(),
                })?;
            camera.is_active = false;
            return Ok(());
        }

        let target = add_target_image(&mut self.app, size, self.pixel_format);
        let world = self.app.world_mut();
        {
            let mut target_component =
                world.get_mut::<RenderTarget>(self.camera).ok_or_else(|| {
                    RendererError::RenderingFailed {
                        message: "the internal render target is unavailable".to_owned(),
                    }
                })?;
            *target_component = RenderTarget::Image(target.clone().into());
        }
        let mut camera =
            world
                .get_mut::<Camera>(self.camera)
                .ok_or_else(|| RendererError::RenderingFailed {
                    message: "the internal camera is unavailable".to_owned(),
                })?;
        camera.is_active = true;
        self.target = target;
        Ok(())
    }

    fn orbit(&mut self, delta_x: f32, delta_y: f32) -> Result<(), RendererError> {
        self.orbit.yaw += delta_x;
        self.orbit.pitch = (self.orbit.pitch + delta_y).clamp(-1.45, 1.45);
        self.update_camera_transform()
    }

    fn zoom(&mut self, delta: f32) -> Result<(), RendererError> {
        self.orbit.distance = (self.orbit.distance * delta.exp())
            .clamp(self.orbit.minimum_distance, self.orbit.maximum_distance);
        self.update_camera_transform()
    }

    fn reset_camera(&mut self) -> Result<(), RendererError> {
        self.orbit = self.initial_orbit;
        self.update_camera_transform()
    }

    fn set_selected_material_slot(&mut self, slot_id: Option<MaterialSlotId>) -> bool {
        let changed = self
            .app
            .world_mut()
            .resource_mut::<SelectionGeometry>()
            .set_selected_slot(slot_id);
        if changed {
            // Gizmos are materialized in Bevy's Last schedule. Run one update
            // before scheduling the readback so the first frame after a
            // selection change contains the newly generated outline asset.
            self.app.update();
        }
        changed
    }

    fn set_selected_primitives(&mut self, primitive_indices: Vec<usize>) -> bool {
        let changed = self
            .app
            .world_mut()
            .resource_mut::<SelectionGeometry>()
            .set_selected_primitives(primitive_indices);
        if changed {
            // Gizmos are materialized in Bevy's Last schedule. Run one update
            // before scheduling the readback so the first frame after a
            // selection change contains the newly generated outline asset.
            self.app.update();
        }
        changed
    }

    fn pick_viewport(
        &mut self,
        x: f32,
        y: f32,
        selection_action: ViewportSelectionAction,
    ) -> Result<(), RendererError> {
        // Camera projection and GlobalTransform are updated by Bevy during the
        // app update. Picking can arrive immediately after a resize or orbit
        // command, so make sure the ray uses the latest camera state.
        self.app.update();

        let (source, picked, request_id) = {
            let world = self.app.world();
            let geometry = world.resource::<SelectionGeometry>();
            let source = geometry.scene_source.clone();
            let picked = world
                .get::<Camera>(self.camera)
                .zip(world.get::<GlobalTransform>(self.camera))
                .and_then(|(camera, transform)| {
                    camera
                        .viewport_to_world(transform, Vec2::new(x, y))
                        .ok()
                        .and_then(|ray| geometry.pick(ray))
                });
            (source, picked, self.scene_request_id)
        };

        let (Some(source), Some(request_id)) = (source, request_id) else {
            return Ok(());
        };
        self.events
            .send(WorkerEvent::Notification(
                RendererNotification::ViewportPickResult {
                    request_id,
                    source,
                    slot_id: picked.map(|picked| picked.slot_id),
                    primitive_index: picked.map(|picked| picked.primitive_index),
                    selection_action,
                },
            ))
            .map_err(|_| RendererError::WorkerStopped)
    }

    fn load_pmx(&mut self, request: PmxLoadRequest) -> Result<bool, RendererError> {
        let requested_source = request.source_identity();
        let request_id = request
            .request_id()
            .unwrap_or_else(|| self.allocate_load_request_id());
        self.next_load_request_id = self
            .next_load_request_id
            .max(request_id.saturating_add(1).max(1));
        self.latest_load_request_id = Some(request_id);
        self.pending_load_tasks += 1;

        let load_events = self.load_event_sender.clone();
        let task_source = requested_source.clone();
        let task = thread::Builder::new()
            .name(format!("charme-pmx-load-{request_id}"))
            .spawn(move || {
                let _ = load_events.send(LoadTaskEvent::Progress(PmxLoadProgress::new(
                    request_id,
                    task_source.clone(),
                    PmxLoadStage::ReadingPmx,
                    None,
                    None,
                )));
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let resolved = request
                        .resolve()
                        .map_err(|message| (task_source.clone(), message))?;
                    let source = resolved.source.identity().clone();
                    let progress_source = source.clone();
                    let prepared = prepare_pmx_scene(&resolved, |stage, completed, total| {
                        let _ = load_events.send(LoadTaskEvent::Progress(PmxLoadProgress::new(
                            request_id,
                            progress_source.clone(),
                            stage,
                            completed,
                            total,
                        )));
                    })
                    .map_err(|message| (source.clone(), message))?;
                    Ok::<_, (PmxSourceIdentity, String)>((source, prepared))
                }));

                match result {
                    Ok(Ok((_, prepared))) => {
                        let _ = load_events.send(LoadTaskEvent::Prepared {
                            request_id,
                            prepared: Box::new(prepared),
                        });
                    }
                    Ok(Err((source, message))) => {
                        let _ = load_events.send(LoadTaskEvent::Failed {
                            request_id,
                            source,
                            message,
                        });
                    }
                    Err(_) => {
                        let _ = load_events.send(LoadTaskEvent::Failed {
                            request_id,
                            source: task_source,
                            message: "the PMX loading task terminated unexpectedly".to_owned(),
                        });
                    }
                }
            });

        match task {
            Ok(task) => self.load_workers.push(task),
            Err(error) => {
                self.pending_load_tasks = self.pending_load_tasks.saturating_sub(1);
                let _ = self.events.send(WorkerEvent::Notification(
                    RendererNotification::PmxLoadFailed {
                        request_id,
                        source: requested_source,
                        message: format!("could not start the PMX loading task: {error}"),
                    },
                ));
            }
        }
        Ok(false)
    }

    fn allocate_load_request_id(&mut self) -> u64 {
        let request_id = self.next_load_request_id.max(1);
        self.next_load_request_id = request_id.saturating_add(1).max(1);
        request_id
    }

    fn poll_load_events(&mut self) -> Result<bool, RendererError> {
        let mut dirty = false;
        while let Ok(event) = self.load_events.try_recv() {
            match event {
                LoadTaskEvent::Progress(progress) => {
                    let _ = self.events.send(WorkerEvent::Notification(
                        RendererNotification::PmxLoadProgress(progress),
                    ));
                }
                LoadTaskEvent::Prepared {
                    request_id,
                    prepared,
                } => {
                    self.pending_load_tasks = self.pending_load_tasks.saturating_sub(1);
                    if self.latest_load_request_id != Some(request_id) {
                        continue;
                    }
                    let source = prepared.info.source_identity().clone();
                    match self.install_prepared_scene(request_id, *prepared) {
                        Ok(installed) => dirty |= installed,
                        Err(error) => {
                            let _ = self.events.send(WorkerEvent::Notification(
                                RendererNotification::PmxLoadFailed {
                                    request_id,
                                    source,
                                    message: error.to_string(),
                                },
                            ));
                        }
                    }
                }
                LoadTaskEvent::Failed {
                    request_id,
                    source,
                    message,
                } => {
                    self.pending_load_tasks = self.pending_load_tasks.saturating_sub(1);
                    let _ = self.events.send(WorkerEvent::Notification(
                        RendererNotification::PmxLoadFailed {
                            request_id,
                            source,
                            message,
                        },
                    ));
                }
            }
        }
        self.reap_finished_load_tasks();
        Ok(dirty)
    }

    fn reap_finished_load_tasks(&mut self) {
        let mut active = Vec::with_capacity(self.load_workers.len());
        for task in self.load_workers.drain(..) {
            if task.is_finished() {
                let _ = task.join();
            } else {
                active.push(task);
            }
        }
        self.load_workers = active;
    }

    fn install_prepared_scene(
        &mut self,
        request_id: u64,
        prepared: PreparedPmxScene,
    ) -> Result<bool, RendererError> {
        if self.latest_load_request_id != Some(request_id) {
            return Ok(false);
        }

        let scene_source = prepared.info.source_identity().clone();
        let progress_events = self.events.clone();
        let progress_source = scene_source.clone();
        let selection =
            SelectionGeometry::from_prepared_with_progress(&prepared, |completed, total| {
                let _ = progress_events.send(WorkerEvent::Notification(
                    RendererNotification::PmxLoadProgress(PmxLoadProgress::new(
                        request_id,
                        progress_source.clone(),
                        PmxLoadStage::BuildingSelection,
                        Some(completed),
                        Some(total),
                    )),
                ));
            });

        let _ = self.events.send(WorkerEvent::Notification(
            RendererNotification::PmxLoadProgress(PmxLoadProgress::new(
                request_id,
                scene_source.clone(),
                PmxLoadStage::BuildingScene,
                None,
                None,
            )),
        ));
        self.clear_material_previews();
        for entity in self.placeholder_entities.drain(..) {
            let _ = self.app.world_mut().despawn(entity);
        }
        for handle in self.placeholder_materials.drain(..) {
            self.app
                .world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .remove(handle.id());
        }
        if let Some(scene) = self.pmx_scene.take() {
            scene.despawn(&mut self.app);
        }
        self.app.world_mut().insert_resource(selection);
        self.scene_request_id = Some(request_id);

        let progress_events = self.events.clone();
        let progress_source = scene_source.clone();
        self.pmx_scene = Some(spawn_pmx_scene(
            &mut self.app,
            &prepared,
            |completed, total| {
                let _ = progress_events.send(WorkerEvent::Notification(
                    RendererNotification::PmxLoadProgress(PmxLoadProgress::new(
                        request_id,
                        progress_source.clone(),
                        PmxLoadStage::BuildingScene,
                        Some(completed),
                        Some(total),
                    )),
                ));
            },
        ));
        let (bounds_min, bounds_max) = prepared.normalized_bounds();
        self.orbit = OrbitState::framing(bounds_min, bounds_max);
        self.initial_orbit = self.orbit;
        self.update_camera_transform()?;
        if let Some(materials) = self.pmx_scene.as_ref().map(|scene| scene.materials.clone()) {
            self.spawn_material_previews(request_id, &scene_source, &materials);
        }
        let _ = self
            .events
            .send(WorkerEvent::Notification(RendererNotification::PmxLoaded {
                request_id,
                info: prepared.info,
            }));
        Ok(true)
    }

    fn join_load_tasks(&mut self) {
        for task in self.load_workers.drain(..) {
            let _ = task.join();
        }
    }

    fn clear_pmx(&mut self) -> Result<(), RendererError> {
        self.latest_load_request_id = None;
        self.scene_request_id = None;
        self.clear_material_previews();
        if let Some(scene) = self.pmx_scene.take() {
            scene.despawn(&mut self.app);
        }
        self.app
            .world_mut()
            .insert_resource(SelectionGeometry::default());
        self.orbit = OrbitState::default();
        self.initial_orbit = self.orbit;
        self.update_camera_transform()
    }

    fn clear_material_previews(&mut self) {
        self.thumbnail_queue.clear();
        self.material_previews.clear();
        self.requested_inspector_slot = None;
        self.set_preview_material(
            self.thumbnail_preview.object,
            self.thumbnail_preview.fallback_material.clone(),
        );
        self.set_preview_material(
            self.inspector_preview.object,
            self.inspector_preview.fallback_material.clone(),
        );
    }

    fn spawn_material_previews(
        &mut self,
        request_id: u64,
        source: &PmxSourceIdentity,
        materials: &[bevy::prelude::Handle<CharmeMaterial>],
    ) {
        for (slot_index, material) in materials.iter().enumerate() {
            let Some(slot_id) = self
                .pmx_scene
                .as_ref()
                .and_then(|scene| scene.material_slot_ids.get(slot_index))
                .copied()
            else {
                continue;
            };
            self.material_previews.push(MaterialPreview {
                request_id,
                slot_id,
                slot_index,
                source: source.clone(),
                material: material.clone(),
            });
            self.thumbnail_queue.push_back(slot_index);
        }
        self.start_next_thumbnail_readback();
    }

    fn finish_material_thumbnail(&mut self, _slot_index: usize) {
        self.pending_thumbnails = self.pending_thumbnails.saturating_sub(1);
        if let Some(mut camera) = self
            .app
            .world_mut()
            .get_mut::<Camera>(self.thumbnail_preview.camera)
        {
            camera.is_active = false;
        }
    }

    fn finish_material_inspector_preview(&mut self, _slot_index: usize) {
        self.pending_inspector_preview = false;
        if let Some(mut camera) = self
            .app
            .world_mut()
            .get_mut::<Camera>(self.inspector_preview.camera)
        {
            camera.is_active = false;
        }
    }

    fn start_next_thumbnail_readback(&mut self) {
        if self.pending_thumbnails != 0 {
            return;
        }
        let Some(slot_index) = self.thumbnail_queue.pop_front() else {
            return;
        };
        let Some((request_id, source, material)) = self
            .material_previews
            .iter()
            .find(|preview| preview.slot_index == slot_index)
            .map(|preview| {
                (
                    preview.request_id,
                    preview.source.clone(),
                    preview.material.clone(),
                )
            })
        else {
            self.start_next_thumbnail_readback();
            return;
        };
        self.set_preview_material(self.thumbnail_preview.object, material);
        if let Some(mut camera) = self
            .app
            .world_mut()
            .get_mut::<Camera>(self.thumbnail_preview.camera)
        {
            camera.is_active = true;
        }
        self.request_thumbnail_readback(request_id, &source, slot_index);
    }

    fn set_preview_material(
        &mut self,
        object: Entity,
        material: bevy::prelude::Handle<CharmeMaterial>,
    ) {
        if let Some(mut current) = self
            .app
            .world_mut()
            .get_mut::<MeshMaterial3d<CharmeMaterial>>(object)
        {
            current.0 = material;
        }
    }

    fn request_material_inspector_preview(
        &mut self,
        slot_id: Option<MaterialSlotId>,
        slot_index: Option<usize>,
    ) {
        let slot_id = slot_id.or_else(|| {
            slot_index.and_then(|index| {
                self.pmx_scene
                    .as_ref()
                    .and_then(|scene| scene.material_slot_ids.get(index))
                    .copied()
            })
        });
        if let Some(slot_id) = slot_id
            && self
                .material_previews
                .iter()
                .any(|preview| preview.slot_id == slot_id)
        {
            self.requested_inspector_slot = Some(slot_id);
            // The selected Inspector preview has priority over stale thumbnail
            // work queued by an earlier material edit.
            self.thumbnail_queue.clear();
        }
    }

    fn start_inspector_preview_readback(&mut self) {
        if self.pending_inspector_preview {
            return;
        }
        let Some(slot_id) = self.requested_inspector_slot.take() else {
            return;
        };
        let Some((request_id, source, material, slot_index)) = self
            .material_previews
            .iter()
            .find(|preview| preview.slot_id == slot_id)
            .map(|preview| {
                (
                    preview.request_id,
                    preview.source.clone(),
                    preview.material.clone(),
                    preview.slot_index,
                )
            })
        else {
            return;
        };
        self.set_preview_material(self.inspector_preview.object, material);
        if let Some(mut camera) = self
            .app
            .world_mut()
            .get_mut::<Camera>(self.inspector_preview.camera)
        {
            camera.is_active = true;
        }
        self.request_inspector_preview_readback(request_id, &source, slot_id, slot_index);
    }

    fn request_thumbnail_readback(
        &mut self,
        request_id: u64,
        source: &PmxSourceIdentity,
        slot_index: usize,
    ) {
        let Some(slot_id) = self
            .pmx_scene
            .as_ref()
            .and_then(|scene| scene.material_slot_ids.get(slot_index))
            .copied()
        else {
            return;
        };
        let events = self.events.clone();
        let completion = self.completion.clone();
        let source = source.clone();
        self.pending_thumbnails += 1;
        self.app
            .world_mut()
            .spawn(Readback::texture(self.thumbnail_preview.target.clone()))
            .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                let frame = Frame::new(
                    slot_index as u64,
                    OutputSize::new(MATERIAL_PREVIEW_SIZE, MATERIAL_PREVIEW_SIZE),
                    PixelFormat::Bgra8Srgb,
                    aligned_bytes_per_row(MATERIAL_PREVIEW_SIZE),
                    event.data.clone(),
                );
                let _ = events.send(WorkerEvent::Notification(
                    RendererNotification::MaterialThumbnailReady {
                        request_id,
                        source: source.clone(),
                        slot_id,
                        slot_index,
                        frame,
                    },
                ));
                let _ = completion.send(Completion::MaterialThumbnail { slot_index });
                commands.entity(event.entity).despawn();
            });
    }

    fn set_material_parameter(
        &mut self,
        slot_id: Option<MaterialSlotId>,
        path: String,
        value: ParameterValue,
    ) -> bool {
        let mut changed = false;
        let handles = match slot_id {
            Some(slot_id) => self
                .pmx_scene
                .as_ref()
                .and_then(|scene| scene.material_for_slot(slot_id))
                .into_iter()
                .collect::<Vec<_>>(),
            None => self.placeholder_materials.iter().collect::<Vec<_>>(),
        };
        if handles.is_empty() {
            let _ = self.events.send(WorkerEvent::Notification(
                RendererNotification::MaterialParameterRejected {
                    path,
                    message: "the selected material slot is not available".to_owned(),
                },
            ));
            return false;
        }
        for handle in handles {
            let result = self
                .app
                .world_mut()
                .resource_mut::<Assets<CharmeMaterial>>()
                .get_mut(handle.id())
                .ok_or_else(|| ParameterError::Unknown {
                    path: "material handle".to_owned(),
                })
                .and_then(|mut material| material.set_parameter(&path, &value));
            match result {
                Ok(()) => changed = true,
                Err(error) => {
                    let _ = self.events.send(WorkerEvent::Notification(
                        RendererNotification::MaterialParameterRejected {
                            path: path.clone(),
                            message: error.to_string(),
                        },
                    ));
                    return false;
                }
            }
        }
        if changed {
            self.refresh_material_previews();
        }
        changed
    }

    fn refresh_material_previews(&mut self) {
        self.thumbnail_queue.clear();
        self.thumbnail_queue.extend(
            self.material_previews
                .iter()
                .map(|preview| preview.slot_index),
        );
        self.start_next_thumbnail_readback();
    }

    fn request_inspector_preview_readback(
        &mut self,
        request_id: u64,
        source: &PmxSourceIdentity,
        slot_id: MaterialSlotId,
        slot_index: usize,
    ) {
        let events = self.events.clone();
        let completion = self.completion.clone();
        let source = source.clone();
        self.pending_inspector_preview = true;
        self.app
            .world_mut()
            .spawn(Readback::texture(self.inspector_preview.target.clone()))
            .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                let frame = Frame::new(
                    slot_index as u64,
                    OutputSize::new(
                        MATERIAL_INSPECTOR_PREVIEW_SIZE,
                        MATERIAL_INSPECTOR_PREVIEW_SIZE,
                    ),
                    PixelFormat::Bgra8Srgb,
                    aligned_bytes_per_row(MATERIAL_INSPECTOR_PREVIEW_SIZE),
                    event.data.clone(),
                );
                let _ = events.send(WorkerEvent::Notification(
                    RendererNotification::MaterialInspectorPreviewReady {
                        request_id,
                        source: source.clone(),
                        slot_id,
                        slot_index,
                        frame,
                    },
                ));
                let _ = completion.send(Completion::MaterialInspectorPreview { slot_index });
                commands.entity(event.entity).despawn();
            });
    }

    fn update_camera_transform(&mut self) -> Result<(), RendererError> {
        let mut transform = self
            .app
            .world_mut()
            .get_mut::<Transform>(self.camera)
            .ok_or_else(|| RendererError::RenderingFailed {
                message: "the internal camera transform is unavailable".to_owned(),
            })?;
        *transform = self.orbit.transform();
        Ok(())
    }

    fn set_background(&mut self, background: BackgroundColor) -> Result<(), RendererError> {
        let mut camera = self
            .app
            .world_mut()
            .get_mut::<Camera>(self.camera)
            .ok_or_else(|| RendererError::RenderingFailed {
                message: "the internal camera is unavailable".to_owned(),
            })?;
        camera.clear_color = ClearColorConfig::Custom(Color::linear_rgb(
            background.red,
            background.green,
            background.blue,
        ));
        Ok(())
    }

    fn request_readback(&mut self) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;

        let size = self.size;
        let pixel_format = self.pixel_format;
        let bytes_per_row = aligned_bytes_per_row(size.width);
        let events = self.events.clone();
        let completion = self.completion.clone();

        self.app
            .world_mut()
            .spawn(Readback::texture(self.target.clone()))
            .observe(move |event: On<ReadbackComplete>, mut commands: Commands| {
                let frame = Frame::new(
                    sequence,
                    size,
                    pixel_format,
                    bytes_per_row,
                    event.data.clone(),
                );
                let _ = events.send(WorkerEvent::Frame(frame));
                let _ = completion.send(Completion::Frame);
                commands.entity(event.entity).despawn();
            });
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.join_load_tasks();
    }
}

fn add_target_image(
    app: &mut App,
    size: OutputSize,
    pixel_format: PixelFormat,
) -> bevy::prelude::Handle<Image> {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture_format(pixel_format),
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn add_preview_target(app: &mut App, size: u32) -> bevy::prelude::Handle<Image> {
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn new_checker_image() -> Image {
    let mut checker = Image::new_fill(
        Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[
            40, 40, 40, 255, 100, 100, 100, 255, 100, 100, 100, 255, 40, 40, 40, 255,
        ],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    checker.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..Default::default()
    });
    checker
}

fn new_preview_floor_mesh() -> Mesh {
    let mut plane = Plane3d {
        half_size: Vec2::splat(4.0),
        ..Default::default()
    }
    .mesh()
    .build();
    if let Some(VertexAttributeValues::Float32x2(uvs)) = plane.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        for uv in uvs.iter_mut() {
            *uv = [uv[0] * 8.0, uv[1] * 8.0];
        }
    }
    plane
}

fn material_preview_camera_transform(offset: Vec3) -> Transform {
    Transform::from_translation(offset + Vec3::new(0.0, 1.5, 2.5)).looking_at(offset, Vec3::Y)
}

fn spawn_material_preview_studio(
    app: &mut App,
    mesh: bevy::prelude::Handle<Mesh>,
    size: u32,
    layer: usize,
    with_floor: bool,
) -> MaterialPreviewStudio {
    let target = add_preview_target(app, size);
    let fallback_material = app
        .world_mut()
        .resource_mut::<Assets<CharmeMaterial>>()
        .add(CharmeMaterial::default());
    let render_layers = RenderLayers::layer(layer);
    let camera_transform = material_preview_camera_transform(Vec3::ZERO);
    let total_distance = camera_transform.translation.length();
    let world = app.world_mut();
    let object = world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(fallback_material.clone()),
            Transform::default(),
            render_layers.clone(),
        ))
        .id();
    let floor = with_floor.then(|| {
        let floor_mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(new_preview_floor_mesh());
        let floor_texture = world
            .resource_mut::<Assets<Image>>()
            .add(new_checker_image());
        let floor_material =
            world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color_texture: Some(floor_texture),
                    perceptual_roughness: 0.9,
                    ..Default::default()
                });
        world
            .spawn((
                Mesh3d(floor_mesh),
                MeshMaterial3d(floor_material),
                Transform::from_xyz(0.0, -1.0, 0.0),
                render_layers.clone(),
            ))
            .id()
    });
    let camera = world
        .spawn((
            Camera3d::default(),
            Tonemapping::None,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                is_active: false,
                ..Default::default()
            },
            Projection::Perspective(bevy::prelude::PerspectiveProjection {
                far: total_distance + 2.0,
                near: (total_distance - 2.0).max(0.1),
                aspect_ratio: 1.0,
                ..Default::default()
            }),
            RenderTarget::Image(target.clone().into()),
            camera_transform,
            render_layers.clone(),
        ))
        .id();
    let key_light = world
        .spawn((
            bevy::prelude::PointLight {
                intensity: 1_200_000.0,
                shadow_maps_enabled: true,
                ..Default::default()
            },
            Transform::from_xyz(4.0, 4.0, 2.0),
            render_layers.clone(),
        ))
        .id();
    let fill_light = world
        .spawn((
            bevy::prelude::PointLight {
                intensity: 400_000.0,
                ..Default::default()
            },
            Transform::from_xyz(-4.0, 2.0, -2.0),
            render_layers,
        ))
        .id();

    MaterialPreviewStudio {
        target,
        object,
        camera,
        _floor: floor,
        _lights: [key_light, fill_light],
        fallback_material,
    }
}

#[derive(Clone, Copy)]
struct OrbitState {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    minimum_distance: f32,
    maximum_distance: f32,
}

impl Default for OrbitState {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.75, 0.0),
            yaw: -0.55,
            pitch: -0.35,
            distance: 7.0,
            minimum_distance: 2.5,
            maximum_distance: 16.0,
        }
    }
}

impl OrbitState {
    fn framing(bounds_min: Vec3, bounds_max: Vec3) -> Self {
        let extent = bounds_max - bounds_min;
        let radius = (extent * 0.5).length().max(0.1);
        let distance = radius * 2.8;
        Self {
            target: (bounds_min + bounds_max) * 0.5,
            distance,
            minimum_distance: (radius * 0.2).max(0.01),
            maximum_distance: (radius * 20.0).max(distance),
            ..Self::default()
        }
    }

    fn transform(self) -> Transform {
        let rotation = Quat::from_euler(bevy::math::EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        Transform::from_translation(self.target + rotation * Vec3::new(0.0, 0.0, self.distance))
            .looking_at(self.target, Vec3::Y)
    }
}

#[derive(Component)]
struct MainPreviewCamera;

const SELECTION_GIZMO_COLOR: Color = Color::srgba(1.0, 0.42, 0.02, 1.0);

fn draw_selected_primitive_gizmo(
    selection: Res<SelectionGeometry>,
    cameras: bevy::prelude::Query<&GlobalTransform, bevy::prelude::With<MainPreviewCamera>>,
    mut gizmos: Gizmos,
) {
    let Some(camera_transform) = cameras.iter().next() else {
        return;
    };
    let camera_position = camera_transform.translation();
    let selected_primitives = selection.selected_primitives();
    let selected_slot = selection.selected_slot();
    if selected_primitives.is_empty() && selected_slot.is_none() {
        return;
    }
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
                    let mut has_front_face = false;
                    let mut has_back_face = false;
                    for &face_index in &edge.faces {
                        let Some(face) = component.faces.get(face_index) else {
                            continue;
                        };
                        if face.normal == Vec3::ZERO {
                            continue;
                        }
                        if face.normal.dot(camera_position - face.center) > 0.0 {
                            has_front_face = true;
                        } else {
                            has_back_face = true;
                        }
                    }
                    has_front_face && has_back_face
                };

                if draw_edge {
                    gizmos.line(edge.start, edge.end, SELECTION_GIZMO_COLOR);
                }
            }
        }
    }
}

fn spawn_scene(
    app: &mut App,
    target: bevy::prelude::Handle<Image>,
    background: BackgroundColor,
    active: bool,
    orbit: OrbitState,
) -> (
    Entity,
    Vec<Entity>,
    Vec<bevy::prelude::Handle<CharmeMaterial>>,
) {
    let cube_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.6, 1.6, 1.6));
    let floor_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Plane3d::default().mesh().size(10.0, 10.0));
    let cube_material = app
        .world_mut()
        .resource_mut::<Assets<CharmeMaterial>>()
        .add(CharmeMaterial::with_tint([0.24, 0.48, 0.95, 1.0]));
    let floor_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.20, 0.24),
            metallic: 0.0,
            perceptual_roughness: 0.85,
            ..Default::default()
        });

    let world = app.world_mut();
    let placeholder = world
        .spawn((
            Mesh3d(cube_mesh),
            MeshMaterial3d(cube_material.clone()),
            Transform::from_xyz(0.0, 0.8, 0.0).with_rotation(Quat::from_euler(
                bevy::math::EulerRot::XYZ,
                0.15,
                0.35,
                0.05,
            )),
        ))
        .id();
    world.spawn((
        Mesh3d(floor_mesh),
        MeshMaterial3d(floor_material),
        Transform::default(),
    ));
    world.spawn((
        DirectionalLight {
            illuminance: 20_000.0,
            shadow_maps_enabled: true,
            ..Default::default()
        },
        Transform::from_rotation(Quat::from_euler(bevy::math::EulerRot::XYZ, -1.0, 0.9, 0.0)),
    ));

    let camera = world
        .spawn((
            MainPreviewCamera,
            Camera3d::default(),
            Tonemapping::None,
            RenderTarget::Image(target.into()),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::linear_rgb(
                    background.red,
                    background.green,
                    background.blue,
                )),
                is_active: active,
                ..Default::default()
            },
            orbit.transform(),
        ))
        .id();
    (camera, vec![placeholder], vec![cube_material])
}

const fn usable_texture_size(size: OutputSize) -> OutputSize {
    OutputSize::new(
        if size.width == 0 { 1 } else { size.width },
        if size.height == 0 { 1 } else { size.height },
    )
}

const fn aligned_bytes_per_row(width: u32) -> usize {
    const ALIGNMENT: usize = 256;
    let unaligned = width as usize * 4;
    unaligned.div_ceil(ALIGNMENT) * ALIGNMENT
}

const fn texture_format(pixel_format: PixelFormat) -> TextureFormat {
    match pixel_format {
        PixelFormat::Bgra8Srgb => TextureFormat::Bgra8UnormSrgb,
        PixelFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_alignment_matches_copy_requirements() {
        assert_eq!(aligned_bytes_per_row(1), 256);
        assert_eq!(aligned_bytes_per_row(64), 256);
        assert_eq!(aligned_bytes_per_row(65), 512);
    }

    #[test]
    fn empty_sizes_get_a_valid_internal_texture() {
        assert_eq!(
            usable_texture_size(OutputSize::new(0, 0)),
            OutputSize::new(1, 1)
        );
        assert_eq!(
            usable_texture_size(OutputSize::new(0, 5)),
            OutputSize::new(1, 5)
        );
    }
}
