use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use crate::model::{RuntimeGraph, ServiceId};
use crate::reducer::EventEnvelope;

#[derive(Clone, Debug)]
pub enum EngineCommand {
    Start { id: ServiceId },
    Stop { id: ServiceId },
    Restart { id: ServiceId },
    Kill { id: ServiceId },
    Toggle { id: ServiceId },
    ClearLogs { id: ServiceId },
    Exec { id: ServiceId, cmd: Vec<String> },
    EmitLog { id: ServiceId, text: String },
    Shutdown,
}

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

#[async_trait]
pub trait Engine: Send + Sync {
    async fn run(
        &mut self,
        command_rx: mpsc::Receiver<EngineCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
        graph: RuntimeGraph,
    );

    /// Get the name of this engine implementation
    fn name(&self) -> &'static str;
}
