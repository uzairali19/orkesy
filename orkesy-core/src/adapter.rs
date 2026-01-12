//! Adapter trait for runtime backends
//!
//! Adapters are the pluggable backends that actually manage units.
//! Each adapter knows how to start, stop, and observe a specific kind of unit
//! (OS processes, Docker containers, etc.)

use async_trait::async_trait;
use std::fmt;
use tokio::sync::broadcast;

use crate::unit::{Unit, UnitHealth, UnitMetrics, UnitStatus};

// Re-export LogStream for consumers
pub use crate::state::LogStream;

/// Errors that can occur during adapter operations
#[derive(Clone, Debug)]
pub enum AdapterError {
    /// Unit not found
    NotFound { id: String },
    /// Unit is already running
    AlreadyRunning { id: String },
    /// Unit is already stopped
    AlreadyStopped { id: String },
    /// Failed to spawn process/container
    SpawnFailed { id: String, message: String },
    /// Failed to stop process/container
    StopFailed { id: String, message: String },
    /// Failed to execute command
    ExecFailed { id: String, message: String },
    /// Operation not supported by this adapter
    NotSupported { operation: String },
    /// Generic error
    Other { message: String },
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdapterError::NotFound { id } => write!(f, "unit not found: {}", id),
            AdapterError::AlreadyRunning { id } => write!(f, "unit already running: {}", id),
            AdapterError::AlreadyStopped { id } => write!(f, "unit already stopped: {}", id),
            AdapterError::SpawnFailed { id, message } => {
                write!(f, "failed to spawn {}: {}", id, message)
            }
            AdapterError::StopFailed { id, message } => {
                write!(f, "failed to stop {}: {}", id, message)
            }
            AdapterError::ExecFailed { id, message } => {
                write!(f, "failed to exec in {}: {}", id, message)
            }
            AdapterError::NotSupported { operation } => {
                write!(f, "operation not supported: {}", operation)
            }
            AdapterError::Other { message } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for AdapterError {}

/// Events emitted by adapters
#[derive(Clone, Debug)]
pub enum AdapterEvent {
    /// Unit status changed
    StatusChanged { id: String, status: UnitStatus },
    /// Unit health changed
    HealthChanged { id: String, health: UnitHealth },
    /// Log line emitted
    LogLine {
        id: String,
        stream: LogStream,
        text: String,
    },
    /// Metrics updated
    MetricsUpdated { id: String, metrics: UnitMetrics },
}

/// Commands that can be sent to an adapter
#[derive(Clone, Debug)]
pub enum AdapterCommand {
    /// Start a unit
    Start { id: String },
    /// Stop a unit
    Stop { id: String },
    /// Restart a unit
    Restart { id: String },
    /// Force kill a unit
    Kill { id: String },
    /// Toggle unit (start if stopped, stop if running)
    Toggle { id: String },
    /// Execute a command in the context of a unit
    Exec { id: String, cmd: Vec<String> },
    /// Clear logs for a unit
    ClearLogs { id: String },
    /// Run install commands for a unit
    Install { id: String },
    /// Shutdown the adapter
    Shutdown,
}

/// The main adapter trait that all runtime backends implement
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Human-readable name of this adapter
    fn name(&self) -> &'static str;

    /// Run the adapter's main loop
    ///
    /// The adapter receives commands via the command channel and emits events
    /// via the event channel. It manages the lifecycle of all units it's responsible for.
    async fn run(
        &mut self,
        command_rx: tokio::sync::mpsc::Receiver<AdapterCommand>,
        event_tx: broadcast::Sender<AdapterEvent>,
        units: Vec<Unit>,
    );

    /// Get the current status of a unit (optional, for query without events)
    fn status(&self, id: &str) -> Option<UnitStatus> {
        let _ = id;
        None
    }

    /// Get current metrics for a unit (optional)
    fn metrics(&self, id: &str) -> Option<UnitMetrics> {
        let _ = id;
        None
    }
}

/// A registry that routes commands to the appropriate adapter based on unit kind
pub struct AdapterRegistry {
    pub process: Option<Box<dyn Adapter>>,
    pub docker: Option<Box<dyn Adapter>>,
    pub generic: Option<Box<dyn Adapter>>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self {
            process: None,
            docker: None,
            generic: None,
        }
    }
}
