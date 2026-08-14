use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    thread::JoinHandle,
};

use charme_core::ParameterValue;

use crate::{
    BackgroundColor, Frame, OutputSize, PmxSceneInfo, RendererConfig, RendererError,
    backend::{self, Command, WorkerEvent},
};

/// A non-frame event produced by the renderer worker.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum RendererNotification {
    /// A PMX model was loaded and installed in the preview scene.
    PmxLoaded(PmxSceneInfo),
    /// A PMX model could not be loaded; the previous scene remains active.
    PmxLoadFailed {
        /// The path that was requested.
        path: PathBuf,
        /// A human-readable import error.
        message: String,
    },
    /// A rendered material-slot thumbnail is ready for the native UI.
    MaterialThumbnailReady {
        /// The PMX scene that owns the thumbnail.
        path: PathBuf,
        /// The zero-based PMX material-slot index.
        slot_index: usize,
        /// The thumbnail pixels, in the renderer's BGRA sRGB format.
        frame: Frame,
    },
    /// A larger material preview with a floor is ready for the Inspector.
    MaterialInspectorPreviewReady {
        /// The PMX scene that owns the preview.
        path: PathBuf,
        /// The zero-based PMX material-slot index.
        slot_index: usize,
        /// The preview pixels, in the renderer's BGRA sRGB format.
        frame: Frame,
    },
    /// A material parameter was rejected without disturbing the current scene.
    MaterialParameterRejected {
        /// The reflected parameter path.
        path: String,
        /// A human-readable validation error.
        message: String,
    },
}

/// A windowless renderer that produces CPU image frames on demand.
///
/// Rendering happens on a private worker. Commands are non-blocking, while
/// [`Renderer::try_recv_frame`] can be polled from a GUI event loop without
/// stalling it.
pub struct Renderer {
    inner: Box<RendererInner>,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Renderer")
            .field("output_size", &self.output_size())
            .finish_non_exhaustive()
    }
}

struct RendererInner {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<WorkerEvent>,
    output_size: Mutex<OutputSize>,
    pending_frame: Option<Frame>,
    notifications: VecDeque<RendererNotification>,
    material_thumbnails: VecDeque<RendererNotification>,
    pending_error: Option<RendererError>,
    disconnected: bool,
    worker: Option<JoinHandle<()>>,
}

impl Renderer {
    /// Creates a renderer and waits for its rendering device to initialize.
    pub fn new(config: RendererConfig) -> Result<Self, RendererError> {
        validate_config(&config)?;

        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let (initialized_tx, initialized_rx) = mpsc::sync_channel(1);
        let size = config.output_size;
        let worker = backend::spawn(config, command_rx, event_tx, initialized_tx)?;

        match initialized_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                inner: Box::new(RendererInner {
                    commands: command_tx,
                    events: event_rx,
                    output_size: Mutex::new(size),
                    pending_frame: None,
                    notifications: VecDeque::new(),
                    material_thumbnails: VecDeque::new(),
                    pending_error: None,
                    disconnected: false,
                    worker: Some(worker),
                }),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(RendererError::InitializationFailed {
                    message: "the rendering worker stopped before initialization completed"
                        .to_owned(),
                })
            }
        }
    }

    /// Returns the most recently requested output size in physical pixels.
    pub fn output_size(&self) -> OutputSize {
        *self
            .inner
            .output_size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Changes the output dimensions and requests a new frame.
    ///
    /// A size with either dimension equal to zero suspends frame production
    /// until a non-empty size is supplied.
    pub fn resize(&self, size: OutputSize) -> Result<(), RendererError> {
        self.send(Command::Resize(size))?;
        *self
            .inner
            .output_size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = size;
        Ok(())
    }

    /// Changes the opaque clear color and requests a new frame.
    pub fn set_background(&self, background: BackgroundColor) -> Result<(), RendererError> {
        validate_background(background)?;
        self.send(Command::SetBackground(background))
    }

    /// Rotates the orbit camera by relative yaw and pitch angles in radians.
    pub fn orbit(&self, delta_x: f32, delta_y: f32) -> Result<(), RendererError> {
        validate_camera_delta(delta_x, delta_y, "orbit deltas must be finite")?;
        self.send(Command::Orbit { delta_x, delta_y })
    }

    /// Changes the camera distance logarithmically.
    ///
    /// Positive values move away from the target; negative values move closer.
    pub fn zoom(&self, delta: f32) -> Result<(), RendererError> {
        if !delta.is_finite() {
            return Err(RendererError::InvalidConfiguration {
                message: "zoom delta must be finite".to_owned(),
            });
        }
        self.send(Command::Zoom(delta))
    }

    /// Restores the camera framing for the current preview scene.
    pub fn reset_camera(&self) -> Result<(), RendererError> {
        self.send(Command::ResetCamera)
    }

    /// Loads a PMX model from an arbitrary file-system path.
    ///
    /// Loading happens on the renderer worker. Completion or failure is
    /// reported through [`Renderer::try_recv_notification`]. Material thumbnails
    /// are read through [`Renderer::try_recv_material_thumbnail`]. A failed load
    /// does not replace the currently displayed scene.
    pub fn load_pmx(&self, path: impl AsRef<Path>) -> Result<(), RendererError> {
        self.send(Command::LoadPmx(path.as_ref().to_path_buf()))
    }

    /// Updates a fixed-ABI material parameter and requests a new frame.
    ///
    /// The path and value are renderer-independent core types. Unsupported
    /// paths or types produce a notification and leave all current materials
    /// unchanged.
    pub fn set_material_parameter(
        &self,
        path: impl Into<String>,
        value: ParameterValue,
    ) -> Result<(), RendererError> {
        if !value.is_finite() {
            return Err(RendererError::InvalidConfiguration {
                message: "material parameters must be finite".to_owned(),
            });
        }
        self.send(Command::SetMaterialParameter {
            path: path.into(),
            value,
        })
    }

    /// Requests a larger, floor-backed preview for one PMX material slot.
    pub fn request_material_inspector_preview(
        &self,
        slot_index: usize,
    ) -> Result<(), RendererError> {
        self.send(Command::RequestMaterialInspectorPreview { slot_index })
    }

    /// Requests a frame representing the latest renderer state.
    ///
    /// Multiple pending requests may be coalesced.
    pub fn request_redraw(&self) -> Result<(), RendererError> {
        self.send(Command::Redraw)
    }

    /// Returns the newest completed frame without blocking.
    ///
    /// Older completed frames are discarded when more than one is waiting.
    pub fn try_recv_frame(&mut self) -> Result<Option<Frame>, RendererError> {
        self.poll_worker_events();
        if let Some(error) = self.inner.pending_error.take() {
            return Err(error);
        }
        if let Some(frame) = self.inner.pending_frame.take() {
            return Ok(Some(frame));
        }
        if self.inner.disconnected {
            return Err(RendererError::WorkerStopped);
        }
        Ok(None)
    }

    /// Returns the oldest pending renderer notification without blocking.
    ///
    /// Material thumbnail results are kept in a separate queue and can be read
    /// with [`Renderer::try_recv_material_thumbnail`].
    pub fn try_recv_notification(&mut self) -> Result<Option<RendererNotification>, RendererError> {
        self.poll_worker_events();
        if let Some(error) = self.inner.pending_error.take() {
            return Err(error);
        }
        if let Some(notification) = self.inner.notifications.pop_front() {
            return Ok(Some(notification));
        }
        if self.inner.disconnected {
            return Err(RendererError::WorkerStopped);
        }
        Ok(None)
    }

    /// Returns the oldest completed material thumbnail without blocking.
    pub fn try_recv_material_thumbnail(
        &mut self,
    ) -> Result<Option<RendererNotification>, RendererError> {
        self.poll_worker_events();
        if let Some(error) = self.inner.pending_error.take() {
            return Err(error);
        }
        if let Some(thumbnail) = self.inner.material_thumbnails.pop_front() {
            return Ok(Some(thumbnail));
        }
        if self.inner.disconnected {
            return Err(RendererError::WorkerStopped);
        }
        Ok(None)
    }

    /// Stops the worker and waits for all private rendering resources to be released.
    pub fn shutdown(mut self) -> Result<(), RendererError> {
        self.stop()
    }

    fn send(&self, command: Command) -> Result<(), RendererError> {
        self.inner
            .commands
            .send(command)
            .map_err(|_| RendererError::WorkerStopped)
    }

    fn poll_worker_events(&mut self) {
        loop {
            match self.inner.events.try_recv() {
                Ok(WorkerEvent::Frame(frame)) => self.inner.pending_frame = Some(frame),
                Ok(WorkerEvent::Notification(notification)) => {
                    if matches!(
                        &notification,
                        RendererNotification::MaterialThumbnailReady { .. }
                    ) {
                        self.inner.material_thumbnails.push_back(notification);
                    } else {
                        self.inner.notifications.push_back(notification);
                    }
                }
                Ok(WorkerEvent::Error(error)) => self.inner.pending_error = Some(error),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.inner.disconnected = true;
                    break;
                }
            }
        }
    }

    fn stop(&mut self) -> Result<(), RendererError> {
        let Some(worker) = self.inner.worker.take() else {
            return Ok(());
        };

        let _ = self.inner.commands.send(Command::Shutdown);
        worker.join().map_err(|_| RendererError::WorkerPanicked)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn validate_config(config: &RendererConfig) -> Result<(), RendererError> {
    validate_background(config.background)
}

fn validate_camera_delta(first: f32, second: f32, message: &str) -> Result<(), RendererError> {
    if !first.is_finite() || !second.is_finite() {
        return Err(RendererError::InvalidConfiguration {
            message: message.to_owned(),
        });
    }
    Ok(())
}

fn validate_background(background: BackgroundColor) -> Result<(), RendererError> {
    if !background.is_valid() {
        return Err(RendererError::InvalidConfiguration {
            message: "background components must be finite and between 0 and 1".to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BackgroundColor;

    #[test]
    fn rejects_non_finite_background_components() {
        let config = RendererConfig::default().background(BackgroundColor::rgb(f32::NAN, 0.0, 0.0));

        assert!(matches!(
            validate_config(&config),
            Err(RendererError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn accepts_zero_sized_output_for_suspension() {
        let config = RendererConfig::new(0, 0);
        assert_eq!(validate_config(&config), Ok(()));
    }

    #[test]
    fn rejects_invalid_runtime_backgrounds() {
        assert!(matches!(
            validate_background(BackgroundColor::rgb(0.0, 2.0, 0.0)),
            Err(RendererError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn rejects_non_finite_camera_input_before_sending_it() {
        let error = RendererError::InvalidConfiguration {
            message: "orbit deltas must be finite".to_owned(),
        };
        assert_eq!(
            validate_camera_delta(f32::NAN, 0.0, "orbit deltas must be finite"),
            Err(error)
        );
    }
}
