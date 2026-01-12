use async_trait::async_trait;
use std::fmt;
use tokio::sync::broadcast;

use crate::unit::{Unit, UnitHealth, UnitMetrics, UnitStatus};

pub use crate::state::LogStream;

#[derive(Clone, Debug)]
pub enum AdapterError {
    NotFound { id: String },
    AlreadyRunning { id: String },
    AlreadyStopped { id: String },
    SpawnFailed { id: String, message: String },
    StopFailed { id: String, message: String },
    ExecFailed { id: String, message: String },
    NotSupported { operation: String },
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

#[derive(Clone, Debug)]
pub enum AdapterEvent {
    StatusChanged { id: String, status: UnitStatus },
    HealthChanged { id: String, health: UnitHealth },
    LogLine {
        id: String,
        stream: LogStream,
        text: String,
    },
    MetricsUpdated { id: String, metrics: UnitMetrics },
}

#[derive(Clone, Debug)]
pub enum AdapterCommand {
    Start { id: String },
    Stop { id: String },
    Restart { id: String },
    Kill { id: String },
    Toggle { id: String },
    Exec { id: String, cmd: Vec<String> },
    ClearLogs { id: String },
    Install { id: String },
    Shutdown,
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn run(
        &mut self,
        command_rx: tokio::sync::mpsc::Receiver<AdapterCommand>,
        event_tx: broadcast::Sender<AdapterEvent>,
        units: Vec<Unit>,
    );

    fn status(&self, id: &str) -> Option<UnitStatus> {
        let _ = id;
        None
    }

    fn metrics(&self, id: &str) -> Option<UnitMetrics> {
        let _ = id;
        None
    }
}

#[derive(Default)]
pub struct AdapterRegistry {
    pub process: Option<Box<dyn Adapter>>,
    pub docker: Option<Box<dyn Adapter>>,
    pub generic: Option<Box<dyn Adapter>>,
}
