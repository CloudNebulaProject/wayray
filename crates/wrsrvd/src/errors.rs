use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum WayRayError {
    #[error("failed to initialize Winit backend")]
    #[diagnostic(help("ensure a display server (X11 or Wayland) is running"))]
    BackendInit(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("failed to initialize Wayland display")]
    #[diagnostic(help("check that the XDG_RUNTIME_DIR environment variable is set"))]
    DisplayInit(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("event loop error")]
    EventLoop(#[source] Box<dyn std::error::Error + Send + Sync>),
}
