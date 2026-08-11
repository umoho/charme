use std::fmt;

/// An error produced while creating, driving, or stopping a renderer.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererError {
    /// The supplied configuration is invalid.
    InvalidConfiguration {
        /// A human-readable description of the invalid value.
        message: String,
    },
    /// The rendering service could not be initialized.
    InitializationFailed {
        /// A human-readable description of the failure.
        message: String,
    },
    /// No compatible rendering device is available.
    DeviceUnavailable,
    /// Rendering a frame failed.
    RenderingFailed {
        /// A human-readable description of the failure.
        message: String,
    },
    /// Transferring a rendered frame to CPU memory failed.
    ReadbackFailed {
        /// A human-readable description of the failure.
        message: String,
    },
    /// The rendering worker is no longer running.
    WorkerStopped,
    /// The rendering worker terminated unexpectedly.
    WorkerPanicked,
}

impl fmt::Display for RendererError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration { message } => {
                write!(formatter, "invalid renderer configuration: {message}")
            }
            Self::InitializationFailed { message } => {
                write!(formatter, "renderer initialization failed: {message}")
            }
            Self::DeviceUnavailable => {
                formatter.write_str("no compatible rendering device is available")
            }
            Self::RenderingFailed { message } => write!(formatter, "rendering failed: {message}"),
            Self::ReadbackFailed { message } => {
                write!(formatter, "frame readback failed: {message}")
            }
            Self::WorkerStopped => formatter.write_str("rendering worker has stopped"),
            Self::WorkerPanicked => formatter.write_str("rendering worker terminated unexpectedly"),
        }
    }
}

impl std::error::Error for RendererError {}
