use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use crate::model::{RuntimeGraph, ServiceId};
use crate::reducer::EventEnvelope;

/// Commands that can be sent to an engine to control services
#[derive(Clone, Debug)]
pub enum EngineCommand {
    /// Start a service
    Start { id: ServiceId },
    /// Gracefully stop a service
    Stop { id: ServiceId },
    /// Restart a service (stop + start)
    Restart { id: ServiceId },
    /// Force kill a service immediately
    Kill { id: ServiceId },
    /// Toggle service state (start if stopped, stop if running)
    Toggle { id: ServiceId },
    /// Clear logs for a service
    ClearLogs { id: ServiceId },
    /// Execute a command in the context of a service
    Exec { id: ServiceId, cmd: Vec<String> },
    /// Emit a log line (for testing/debugging)
    EmitLog { id: ServiceId, text: String },
    /// Shutdown the engine
    Shutdown,
}

/// Result of an engine operation
#[derive(Clone, Debug)]
pub enum EngineError {
    ServiceNotFound { id: ServiceId },
    AlreadyRunning { id: ServiceId },
    AlreadyStopped { id: ServiceId },
    SpawnFailed { id: ServiceId, reason: String },
    KillFailed { id: ServiceId, reason: String },
    NotSupported { operation: String },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceNotFound { id } => write!(f, "service not found: {}", id),
            Self::AlreadyRunning { id } => write!(f, "service already running: {}", id),
            Self::AlreadyStopped { id } => write!(f, "service already stopped: {}", id),
            Self::SpawnFailed { id, reason } => write!(f, "failed to spawn {}: {}", id, reason),
            Self::KillFailed { id, reason } => write!(f, "failed to kill {}: {}", id, reason),
            Self::NotSupported { operation } => write!(f, "operation not supported: {}", operation),
        }
    }
}

impl std::error::Error for EngineError {}

/// The Engine trait defines a pluggable backend for managing services.
///
/// Different implementations can manage services in different ways:
/// - `FakeEngine`: Simulates service lifecycle for demos/testing
/// - `LocalProcessEngine`: Spawns real OS processes
/// - `DockerEngine`: Manages Docker containers
///
/// All engines communicate via channels:
/// - Receive commands via `command_rx`
/// - Emit events (status changes, logs) via `event_tx`
#[async_trait]
pub trait Engine: Send + Sync {
    /// Run the engine's main loop.
    ///
    /// This method should:
    /// 1. Emit `TopologyLoaded` with the initial graph
    /// 2. Process incoming commands from `command_rx`
    /// 3. Emit events (StatusChanged, LogLine, etc.) via `event_tx`
    /// 4. Return when `Shutdown` command is received or channel closes
    async fn run(
        &mut self,
        command_rx: mpsc::Receiver<EngineCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
        graph: RuntimeGraph,
    );

    /// Get the name of this engine implementation
    fn name(&self) -> &'static str;
}
