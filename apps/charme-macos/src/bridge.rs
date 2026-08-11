use std::{
    path::PathBuf,
    sync::mpsc::{self, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

use cacao::appkit::App;
use charme_core::ParameterValue;
use charme_renderer::{BackgroundColor, OutputSize, PixelFormat, Renderer, RendererConfig};

use crate::app::{CharmeApp, Message};

enum Command {
    Resize { size: OutputSize, scale: f64 },
    SetBackground(BackgroundColor),
    Orbit { delta_x: f32, delta_y: f32 },
    Zoom(f32),
    LoadPmx(PathBuf),
    SetMaterialParameter { path: String, value: ParameterValue },
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
                        dispatch(Message::Failed(error.to_string()));
                        return;
                    }
                };
                let mut display_scale = scale;

                if let Err(error) = renderer.request_redraw() {
                    dispatch(Message::Failed(error.to_string()));
                    return;
                }

                'running: loop {
                    loop {
                        match receiver.try_recv() {
                            Ok(Command::Resize { size, scale }) => {
                                display_scale = scale;
                                if let Err(error) = renderer.resize(size) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::SetBackground(background)) => {
                                if let Err(error) = renderer.set_background(background) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Orbit { delta_x, delta_y }) => {
                                if let Err(error) = renderer.orbit(delta_x, delta_y) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Zoom(delta)) => {
                                if let Err(error) = renderer.zoom(delta) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::LoadPmx(path)) => {
                                if let Err(error) = renderer.load_pmx(path) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::SetMaterialParameter { path, value }) => {
                                if let Err(error) = renderer.set_material_parameter(path, value) {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Redraw) => {
                                if let Err(error) = renderer.request_redraw() {
                                    dispatch(Message::Failed(error.to_string()));
                                    break 'running;
                                }
                            }
                            Ok(Command::Stop) => break 'running,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => break 'running,
                        }
                    }

                    match renderer.try_recv_frame() {
                        Ok(Some(frame)) => dispatch(Message::Frame {
                            frame,
                            scale: display_scale,
                        }),
                        Ok(None) => {}
                        Err(error) => {
                            dispatch(Message::Failed(error.to_string()));
                            break;
                        }
                    }
                    loop {
                        match renderer.try_recv_notification() {
                            Ok(Some(notification)) => {
                                dispatch(Message::RendererNotification(notification));
                            }
                            Ok(None) => break,
                            Err(error) => {
                                dispatch(Message::Failed(error.to_string()));
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

    pub(crate) fn set_brightness(&self, value: f32) {
        let value = value.clamp(0.0, 1.0);
        let background = BackgroundColor::rgb(
            0.015 + value * 0.08,
            0.018 + value * 0.09,
            0.025 + value * 0.12,
        );
        let _ = self.commands.send(Command::SetBackground(background));
    }

    pub(crate) fn orbit(&self, delta_x: f32, delta_y: f32) {
        let _ = self.commands.send(Command::Orbit { delta_x, delta_y });
    }

    pub(crate) fn zoom(&self, delta: f32) {
        let _ = self.commands.send(Command::Zoom(delta));
    }

    pub(crate) fn load_pmx(&self, path: PathBuf) {
        let _ = self.commands.send(Command::LoadPmx(path));
    }

    pub(crate) fn set_material_parameter(&self, path: String, value: ParameterValue) {
        let _ = self
            .commands
            .send(Command::SetMaterialParameter { path, value });
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

fn dispatch(message: Message) {
    App::<CharmeApp, Message>::dispatch_main(message);
}
