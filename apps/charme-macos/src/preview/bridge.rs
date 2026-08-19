use std::{
    sync::mpsc::{self, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

use cacao::appkit::App;
use charme_application::ApplicationEvent;
use charme_core::{MaterialSlotId, ParameterValue};
use charme_renderer::{
    BackgroundColor, OutputSize, PixelFormat, PmxLoadRequest, Renderer, RendererConfig,
    ViewportSelectionAction,
};

use crate::{
    app::{CharmeApp, Message},
    localization::{self, Key},
};

enum Command {
    Resize {
        size: OutputSize,
        scale: f64,
    },
    Orbit {
        delta_x: f32,
        delta_y: f32,
    },
    Zoom(f32),
    LoadPmx(PmxLoadRequest),
    ClearPmx,
    SetMaterialParameter {
        slot_id: MaterialSlotId,
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
        slot_id: MaterialSlotId,
    },
    Redraw,
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
                            Ok(Command::Resize { size, scale }) => {
                                display_scale = scale;
                                if let Err(error) = renderer.resize(size) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Orbit { delta_x, delta_y }) => {
                                if let Err(error) = renderer.orbit(delta_x, delta_y) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Zoom(delta)) => {
                                if let Err(error) = renderer.zoom(delta) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::LoadPmx(request)) => {
                                if let Err(error) = renderer.load_pmx_request(request) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::ClearPmx) => {
                                if let Err(error) = renderer.clear_pmx() {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::SetMaterialParameter {
                                slot_id,
                                path,
                                value,
                            }) => {
                                if let Err(error) =
                                    renderer.set_material_parameter_for_slot(slot_id, path, value)
                                {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::SetSelectedMaterialSlot(slot_id)) => {
                                if let Err(error) = renderer.set_selected_material_slot(slot_id) {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::SetSelectedPrimitives(primitive_indices)) => {
                                if let Err(error) =
                                    renderer.set_selected_primitives(primitive_indices)
                                {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::PickViewport {
                                x,
                                y,
                                selection_action,
                            }) => {
                                if let Err(error) =
                                    renderer.pick_viewport_with_action(x, y, selection_action)
                                {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::RequestMaterialInspectorPreview { slot_id }) => {
                                if let Err(error) =
                                    renderer.request_material_inspector_preview_for_slot(slot_id)
                                {
                                    dispatch_event(ApplicationEvent::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Redraw) => {
                                if let Err(error) = renderer.request_redraw() {
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
                    loop {
                        match renderer.try_recv_notification() {
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

    pub(crate) fn resize(&self, size: OutputSize, scale: f64) {
        let _ = self.commands.send(Command::Resize { size, scale });
    }

    pub(crate) fn orbit(&self, delta_x: f32, delta_y: f32) {
        let _ = self.commands.send(Command::Orbit { delta_x, delta_y });
    }

    pub(crate) fn zoom(&self, delta: f32) {
        let _ = self.commands.send(Command::Zoom(delta));
    }

    pub(crate) fn load_pmx(&self, request: PmxLoadRequest) {
        let _ = self.commands.send(Command::LoadPmx(request));
    }

    pub(crate) fn clear_pmx(&self) {
        let _ = self.commands.send(Command::ClearPmx);
    }

    pub(crate) fn set_material_parameter(
        &self,
        slot_id: MaterialSlotId,
        path: String,
        value: ParameterValue,
    ) {
        let _ = self.commands.send(Command::SetMaterialParameter {
            slot_id,
            path,
            value,
        });
    }

    pub(crate) fn request_material_inspector_preview(&self, slot_id: MaterialSlotId) {
        let _ = self
            .commands
            .send(Command::RequestMaterialInspectorPreview { slot_id });
    }

    pub(crate) fn set_selected_material_slot(&self, slot_id: Option<MaterialSlotId>) {
        let _ = self
            .commands
            .send(Command::SetSelectedMaterialSlot(slot_id));
    }

    pub(crate) fn set_selected_primitives(&self, primitive_indices: Vec<usize>) {
        let _ = self
            .commands
            .send(Command::SetSelectedPrimitives(primitive_indices));
    }

    pub(crate) fn pick_viewport(&self, x: f32, y: f32, selection_action: ViewportSelectionAction) {
        let _ = self.commands.send(Command::PickViewport {
            x,
            y,
            selection_action,
        });
    }

    pub(crate) fn request_redraw(&self) {
        let _ = self.commands.send(Command::Redraw);
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
