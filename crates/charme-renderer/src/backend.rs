use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread,
    time::Duration,
};

use bevy::{
    DefaultPlugins,
    app::{App, PluginGroup, PluginsState},
    asset::{Assets, RenderAssetUsages},
    camera::{
        Camera, Camera3d, ClearColorConfig, Projection, RenderTarget, visibility::RenderLayers,
    },
    core_pipeline::tonemapping::Tonemapping,
    image::Image,
    prelude::{
        Color, Commands, Cuboid, DirectionalLight, Entity, Mesh, Mesh3d, MeshMaterial3d, Meshable,
        On, Plane3d, Quat, Sphere, StandardMaterial, Transform, Vec3,
    },
    render::{
        RenderApp, RenderPlugin,
        gpu_readback::{Readback, ReadbackComplete},
        pipelined_rendering::PipelinedRenderingPlugin,
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
};

use charme_bevy::{CharmeMaterial, CharmeMaterialPlugin, ParameterError};
use charme_core::ParameterValue;

use crate::{
    BackgroundColor, Frame, OutputSize, PixelFormat, RendererConfig, RendererError,
    renderer::RendererNotification,
    scene::{SpawnedPmxScene, prepare_pmx_scene, spawn_pmx_scene},
};

pub(crate) enum Command {
    Resize(OutputSize),
    SetBackground(BackgroundColor),
    Orbit { delta_x: f32, delta_y: f32 },
    Zoom(f32),
    ResetCamera,
    LoadPmx(PathBuf),
    SetMaterialParameter { path: String, value: ParameterValue },
    Redraw,
    Shutdown,
}

pub(crate) enum WorkerEvent {
    Frame(Frame),
    Notification(RendererNotification),
    Error(RendererError),
}

enum Completion {
    Frame,
    MaterialThumbnail {
        target: bevy::prelude::Handle<Image>,
        slot_index: usize,
    },
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
    let mut backend = Backend::new(&config, events, completion_tx)?;
    initialized
        .send(Ok(()))
        .map_err(|_| RendererError::WorkerStopped)?;

    let mut dirty = false;
    let mut in_flight = false;

    loop {
        while let Ok(completion) = completion_rx.try_recv() {
            match completion {
                Completion::Frame => in_flight = false,
                Completion::MaterialThumbnail { target, slot_index } => {
                    backend.finish_material_thumbnail(slot_index, target);
                }
            }
        }

        let command = if in_flight
            || dirty
            || backend.pending_thumbnails != 0
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
                Command::LoadPmx(path) => {
                    dirty |= backend.load_pmx(path)?;
                }
                Command::SetMaterialParameter { path, value } => {
                    dirty |= backend.set_material_parameter(path, value);
                }
                Command::Redraw => dirty = true,
                Command::Shutdown => return Ok(()),
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
                Ok(Command::LoadPmx(path)) => {
                    dirty |= backend.load_pmx(path)?;
                }
                Ok(Command::SetMaterialParameter { path, value }) => {
                    dirty |= backend.set_material_parameter(path, value);
                }
                Ok(Command::Redraw) => dirty = true,
                Ok(Command::Shutdown) => return Ok(()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

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
        }

        if in_flight || backend.pending_thumbnails != 0 {
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
    orbit: OrbitState,
    initial_orbit: OrbitState,
    next_sequence: u64,
    events: Sender<WorkerEvent>,
    completion: Sender<Completion>,
    pending_thumbnails: usize,
    thumbnail_mesh: bevy::prelude::Handle<Mesh>,
    material_previews: Vec<MaterialPreview>,
    thumbnail_queue: VecDeque<usize>,
    retired_thumbnail_targets: Vec<bevy::prelude::Handle<Image>>,
}

struct MaterialPreview {
    slot_index: usize,
    target: bevy::prelude::Handle<Image>,
    scene_path: PathBuf,
    camera: Entity,
    entities: Vec<Entity>,
}

const MATERIAL_PREVIEW_SIZE: u32 = 64;
const MATERIAL_PREVIEW_LAYER: usize = 30;

impl Backend {
    fn new(
        config: &RendererConfig,
        events: Sender<WorkerEvent>,
        completion: Sender<Completion>,
    ) -> Result<Self, RendererError> {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..Default::default()
                })
                .build()
                .disable::<PipelinedRenderingPlugin>(),
        );
        app.add_plugins(CharmeMaterialPlugin);

        while app.plugins_state() == PluginsState::Adding {
            thread::yield_now();
        }
        app.finish();
        app.cleanup();

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
            orbit,
            initial_orbit: orbit,
            next_sequence: 1,
            events,
            completion,
            pending_thumbnails: 0,
            thumbnail_mesh,
            material_previews: Vec::new(),
            thumbnail_queue: VecDeque::new(),
            retired_thumbnail_targets: Vec::new(),
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

    fn load_pmx(&mut self, path: PathBuf) -> Result<bool, RendererError> {
        let prepared = match prepare_pmx_scene(&path) {
            Ok(prepared) => prepared,
            Err(message) => {
                let _ = self.events.send(WorkerEvent::Notification(
                    RendererNotification::PmxLoadFailed { path, message },
                ));
                return Ok(false);
            }
        };

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
        let scene_path = prepared.info.path().to_path_buf();
        self.pmx_scene = Some(spawn_pmx_scene(&mut self.app, &prepared));
        let (bounds_min, bounds_max) = prepared.normalized_bounds();
        self.orbit = OrbitState::framing(bounds_min, bounds_max);
        self.initial_orbit = self.orbit;
        self.update_camera_transform()?;
        if let Some(materials) = self.pmx_scene.as_ref().map(|scene| scene.materials.clone()) {
            self.spawn_material_previews(&scene_path, &materials);
        }
        let _ = self
            .events
            .send(WorkerEvent::Notification(RendererNotification::PmxLoaded(
                prepared.info,
            )));
        Ok(true)
    }

    fn clear_material_previews(&mut self) {
        self.thumbnail_queue.clear();
        for preview in self.material_previews.drain(..) {
            for entity in preview.entities {
                let _ = self.app.world_mut().despawn(entity);
            }
            // A readback may still reference this target. Keep the asset alive
            // until its completion callback has been observed.
            self.retired_thumbnail_targets.push(preview.target);
        }
    }

    fn retire_thumbnail_target(&mut self, target: bevy::prelude::Handle<Image>) {
        let Some(index) = self
            .retired_thumbnail_targets
            .iter()
            .position(|candidate| candidate.id() == target.id())
        else {
            return;
        };
        let target = self.retired_thumbnail_targets.swap_remove(index);
        self.app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .remove(target.id());
    }

    fn spawn_material_previews(
        &mut self,
        scene_path: &std::path::Path,
        materials: &[bevy::prelude::Handle<CharmeMaterial>],
    ) {
        let render_layers = RenderLayers::layer(MATERIAL_PREVIEW_LAYER);
        for (slot_index, material) in materials.iter().enumerate() {
            let target = add_thumbnail_target(&mut self.app);
            // Keep studios spatially separated so every preview camera can use
            // the shared preview layer without seeing the other slot spheres.
            let studio_offset = Vec3::new(slot_index as f32 * 10.0, 0.0, 0.0);
            let camera_transform = material_preview_camera_transform(studio_offset);
            let total_distance = (camera_transform.translation - studio_offset).length();
            let mut entities = Vec::with_capacity(4);
            let world = self.app.world_mut();
            let object = world
                .spawn((
                    Mesh3d(self.thumbnail_mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(studio_offset),
                    render_layers.clone(),
                ))
                .id();
            entities.push(object);
            let camera = world
                .spawn((
                    Camera3d::default(),
                    Tonemapping::None,
                    Camera {
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        // Only one thumbnail camera is active at a time. This
                        // prevents dozens of simultaneous GPU readbacks from
                        // overwhelming the device on large PMX models.
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
            entities.push(camera);
            let key_light = world
                .spawn((
                    bevy::prelude::PointLight {
                        intensity: 1_200_000.0,
                        shadow_maps_enabled: true,
                        ..Default::default()
                    },
                    Transform::from_translation(studio_offset + Vec3::new(4.0, 4.0, 2.0)),
                    render_layers.clone(),
                ))
                .id();
            entities.push(key_light);
            let fill_light = world
                .spawn((
                    bevy::prelude::PointLight {
                        intensity: 400_000.0,
                        ..Default::default()
                    },
                    Transform::from_translation(studio_offset + Vec3::new(-4.0, 2.0, -2.0)),
                    render_layers.clone(),
                ))
                .id();
            entities.push(fill_light);

            self.material_previews.push(MaterialPreview {
                slot_index,
                target,
                scene_path: scene_path.to_path_buf(),
                camera,
                entities,
            });
            self.thumbnail_queue.push_back(slot_index);
        }
        self.start_next_thumbnail_readback();
    }

    fn finish_material_thumbnail(
        &mut self,
        slot_index: usize,
        target: bevy::prelude::Handle<Image>,
    ) {
        self.pending_thumbnails = self.pending_thumbnails.saturating_sub(1);
        self.retire_thumbnail_target(target);
        let camera_entity = self
            .material_previews
            .iter()
            .find(|preview| preview.slot_index == slot_index)
            .map(|preview| preview.camera);
        if let Some(camera_entity) = camera_entity
            && let Some(mut camera) = self.app.world_mut().get_mut::<Camera>(camera_entity)
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
        let Some((scene_path, target, camera)) = self
            .material_previews
            .iter()
            .find(|preview| preview.slot_index == slot_index)
            .map(|preview| {
                (
                    preview.scene_path.clone(),
                    preview.target.clone(),
                    preview.camera,
                )
            })
        else {
            self.start_next_thumbnail_readback();
            return;
        };
        if let Some(mut camera) = self.app.world_mut().get_mut::<Camera>(camera) {
            camera.is_active = true;
        }
        self.request_thumbnail_readback(&scene_path, slot_index, target);
    }

    fn request_thumbnail_readback(
        &mut self,
        scene_path: &std::path::Path,
        slot_index: usize,
        target: bevy::prelude::Handle<Image>,
    ) {
        let events = self.events.clone();
        let completion = self.completion.clone();
        let completion_target = target.clone();
        let scene_path = scene_path.to_path_buf();
        self.pending_thumbnails += 1;
        self.app
            .world_mut()
            .spawn(Readback::texture(target))
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
                        path: scene_path.clone(),
                        slot_index,
                        frame,
                    },
                ));
                let _ = completion.send(Completion::MaterialThumbnail {
                    target: completion_target.clone(),
                    slot_index,
                });
                commands.entity(event.entity).despawn();
            });
    }

    fn set_material_parameter(&mut self, path: String, value: ParameterValue) -> bool {
        let mut changed = false;
        let handles = self.placeholder_materials.iter().chain(
            self.pmx_scene
                .iter()
                .flat_map(|scene| scene.materials.iter()),
        );
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

fn add_thumbnail_target(app: &mut App) -> bevy::prelude::Handle<Image> {
    let mut image = Image::new_uninit(
        Extent3d {
            width: MATERIAL_PREVIEW_SIZE,
            height: MATERIAL_PREVIEW_SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    app.world_mut().resource_mut::<Assets<Image>>().add(image)
}

fn material_preview_camera_transform(offset: Vec3) -> Transform {
    Transform::from_translation(offset + Vec3::new(0.0, 1.5, 2.5)).looking_at(offset, Vec3::Y)
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
