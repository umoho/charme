use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

use cacao::appkit::App;
use charme_application::ApplicationEvent;
use charme_core::{MaterialSlotId, ParameterValue};
use charme_renderer::{
    BackgroundColor, OutputSize, PixelFormat, PmxLoadRequest, Renderer, RendererConfig,
    RendererError, RendererNotification, ViewportSelectionAction,
};

use crate::{
    app::{CharmeApp, Message},
    localization::{self, Key},
};

type RendererOperation = Box<dyn FnOnce(&Renderer) -> Result<(), RendererError> + Send + 'static>;

enum Command {
    Execute {
        operation: RendererOperation,
        display_scale: Option<f64>,
    },
    Stop,
}

pub(crate) struct RenderBridge {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl RenderBridge {
    pub(crate) fn start(size: OutputSize, scale: f64) -> Self {
        let (commands, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("charme-ui-render-bridge".to_owned())
            .spawn(move || {
                let config = RendererConfig::new(size.width, size.height)
                    .pixel_format(PixelFormat::Bgra8Srgb)
                    .background(BackgroundColor::rgb(0.035, 0.040, 0.052));
                let mut renderer = match Renderer::new(config) {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        dispatch_event(ApplicationEvent::Failed(error.to_string()));
                        return;
                    }
                };
                let mut display_scale = scale;

                if let Err(error) = renderer.request_redraw() {
                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                    return;
                }

                'running: loop {
                    loop {
                        match receiver.try_recv() {
                            Ok(Command::Execute {
                                operation,
                                display_scale: requested_scale,
                            }) => {
                                if let Some(scale) = requested_scale {
                                    display_scale = scale;
                                }
                                if let Err(error) = operation(&renderer) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Stop) => break 'running,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break 'running,
                        }
                    }

                    match renderer.try_recv_frame() {
                        Ok(Some(frame)) => dispatch_event(ApplicationEvent::FrameReady {
                            frame,
                            scale: display_scale,
                        }),
                        Ok(None) => {}
                        Err(error) => {
                            dispatch_event(ApplicationEvent::Failed(error.to_string()));
                            break;
                        }
                    }
                    let mut latest_progress = None;
                    loop {
                        match renderer.try_recv_notification() {
                            Ok(Some(RendererNotification::PmxLoadProgress(progress))) => {
                                latest_progress =
                                    Some(RendererNotification::PmxLoadProgress(progress));
                            }
                            Ok(Some(notification)) => {
                                if let Some(progress) = latest_progress.take() {
                                    dispatch_event(ApplicationEvent::Renderer(progress));
                                }
                                dispatch_event(ApplicationEvent::Renderer(notification));
                            }
                            Ok(None) => break,
                            Err(error) => {
                                dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                break 'running;
                            }
                        }
                    }
                    if let Some(progress) = latest_progress {
                        dispatch_event(ApplicationEvent::Renderer(progress));
                    }
                    loop {
                        match renderer.try_recv_material_thumbnail() {
                            Ok(Some(notification)) => {
                                dispatch_event(ApplicationEvent::Renderer(notification));
                            }
                            Ok(None) => break,
                            Err(error) => {
                                dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                break 'running;
                            }
                        }
                    }

                    thread::sleep(Duration::from_millis(4));
                }

                let _ = renderer.shutdown();
            })
            .expect("failed to start the Charme renderer bridge");

        Self {
            commands,
            worker: Some(worker),
        }
    }

    fn execute(
        &self,
        display_scale: Option<f64>,
        operation: impl FnOnce(&Renderer) -> Result<(), RendererError> + Send + 'static,
    ) {
        let _ = self.commands.send(Command::Execute {
            operation: Box::new(operation),
            display_scale,
        });
    }

    pub(crate) fn resize(&self, size: OutputSize, scale: f64) {
        self.execute(Some(scale), move |renderer| renderer.resize(size));
    }

    pub(crate) fn orbit(&self, delta_x: f32, delta_y: f32) {
        self.execute(None, move |renderer| renderer.orbit(delta_x, delta_y));
    }

    pub(crate) fn zoom(&self, delta: f32) {
        self.execute(None, move |renderer| renderer.zoom(delta));
    }

    pub(crate) fn load_pmx(&self, request: PmxLoadRequest) {
        self.execute(None, move |renderer| renderer.load_pmx_request(request));
    }

    pub(crate) fn clear_pmx(&self) {
        self.execute(None, Renderer::clear_pmx);
    }

    pub(crate) fn sync_material_parameters(
        &self,
        slot_id: MaterialSlotId,
        parameters: BTreeMap<String, ParameterValue>,
    ) {
        self.execute(None, move |renderer| {
            renderer.sync_material_parameters_for_slot(slot_id, parameters)
        });
    }

    pub(crate) fn request_material_inspector_preview(&self, slot_id: MaterialSlotId) {
        self.execute(None, move |renderer| {
            renderer.request_material_inspector_preview_for_slot(slot_id)
        });
    }

    pub(crate) fn set_selected_material_slot(&self, slot_id: Option<MaterialSlotId>) {
        self.execute(None, move |renderer| {
            renderer.set_selected_material_slot(slot_id)
        });
    }

    pub(crate) fn set_selected_primitives(&self, primitive_indices: Vec<usize>) {
        self.execute(None, move |renderer| {
            renderer.set_selected_primitives(primitive_indices)
        });
    }

    pub(crate) fn split_selected_primitives_by_connectivity(&self, primitive_indices: Vec<usize>) {
        self.execute(None, move |renderer| {
            renderer.split_selected_primitives_by_connectivity(primitive_indices)
        });
    }

    pub(crate) fn pick_viewport(&self, x: f32, y: f32, selection_action: ViewportSelectionAction) {
        self.execute(None, move |renderer| {
            renderer.pick_viewport_with_action(x, y, selection_action)
        });
    }

    pub(crate) fn request_redraw(&self) {
        self.execute(None, Renderer::request_redraw);
    }
}

impl Drop for RenderBridge {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn dispatch_event(event: ApplicationEvent) {
    let event = match event {
        ApplicationEvent::Failed(error) => {
            tracing::error!(error = %error, "Renderer failure");
            ApplicationEvent::Failed(format!(
                "{}: {error}",
                localization::text(Key::RendererFailed)
            ))
        }
        event => event,
    };
    App::<CharmeApp, Message>::dispatch_main(Message::Application(event));
}
