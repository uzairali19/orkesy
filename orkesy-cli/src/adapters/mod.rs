//! Adapter implementations for different runtime backends

mod process;

#[cfg(feature = "docker")]
mod docker;

pub use process::{ProcessAdapter, format_bytes};

#[cfg(feature = "docker")]
pub use docker::DockerAdapter;
