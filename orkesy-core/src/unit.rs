//! Unit model for Orkesy
//!
//! A Unit is the core abstraction in Orkesy - it represents anything that can be
//! started, stopped, and observed: Docker containers, OS processes, shell commands, etc.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Unique identifier for a unit
pub type UnitId = String;

/// The kind of runtime adapter to use for this unit
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    /// OS process spawned directly
    Process,
    /// Docker container
    Docker,
    /// Generic shell command (for glue/scripts)
    Generic,
}

impl Default for UnitKind {
    fn default() -> Self {
        UnitKind::Process
    }
}

/// How to stop a unit
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopBehavior {
    /// Send a signal (SIGINT, SIGTERM, SIGKILL)
    Signal(StopSignal),
    /// Run a command to stop
    Command(String),
}

impl Default for StopBehavior {
    fn default() -> Self {
        StopBehavior::Signal(StopSignal::SigInt)
    }
}

/// Unix signals for stopping processes
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum StopSignal {
    #[serde(alias = "SIGINT", alias = "sigint", alias = "INT")]
    SigInt,
    #[serde(alias = "SIGTERM", alias = "sigterm", alias = "TERM")]
    SigTerm,
    #[serde(alias = "SIGKILL", alias = "sigkill", alias = "KILL")]
    SigKill,
}

/// Health check configuration for a unit
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HealthCheck {
    /// TCP port check
    Tcp {
        #[serde(default = "default_health_port")]
        port: u16,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
    },
    /// HTTP endpoint check
    Http {
        url: String,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
    },
    /// Execute a command
    Exec {
        command: String,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
    },
}

fn default_health_port() -> u16 {
    8000
}
fn default_interval_ms() -> u64 {
    5000
}
fn default_timeout_ms() -> u64 {
    2000
}

/// A Unit definition - the core config for something Orkesy manages
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
    /// Unique identifier
    #[serde(skip)]
    pub id: UnitId,

    /// Human-readable display name
    #[serde(default)]
    pub name: Option<String>,

    /// Kind of unit (process, docker, generic)
    #[serde(default)]
    pub kind: UnitKind,

    /// Working directory for the unit
    #[serde(default)]
    pub cwd: Option<PathBuf>,

    /// Environment variables
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Install commands to run before starting (e.g., "pnpm install", "uv sync")
    #[serde(default)]
    pub install: Vec<String>,

    /// Start command (shell string, not vec)
    pub start: String,

    /// How to stop the unit
    #[serde(default)]
    pub stop: StopBehavior,

    /// Custom log command (for docker units that need "docker compose logs -f")
    #[serde(default)]
    pub logs: Option<String>,

    /// Health check configuration
    #[serde(default)]
    pub health: Option<HealthCheck>,

    /// Description for display
    #[serde(default)]
    pub description: Option<String>,

    /// Port the service listens on (informational)
    #[serde(default)]
    pub port: Option<u16>,

    /// Auto-start when orkesy launches
    #[serde(default = "default_autostart")]
    pub autostart: bool,
}

fn default_autostart() -> bool {
    false
}

impl Unit {
    /// Get the display name (falls back to id)
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Runtime status of a unit
#[derive(Clone, Debug, Default)]
pub enum UnitStatus {
    #[default]
    Unknown,
    Starting,
    Running,
    Stopping,
    Stopped,
    Exited {
        code: Option<i32>,
    },
    Errored {
        message: String,
    },
}

impl UnitStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, UnitStatus::Running)
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self, UnitStatus::Stopped | UnitStatus::Exited { .. })
    }
}

/// Health status of a unit
#[derive(Clone, Debug, Default)]
pub enum UnitHealth {
    #[default]
    Unknown,
    Healthy,
    Degraded {
        reason: String,
    },
    Unhealthy {
        reason: String,
    },
}

/// Runtime metrics for a unit
#[derive(Clone, Debug, Default)]
pub struct UnitMetrics {
    /// CPU usage as percentage (0.0 - 100.0)
    pub cpu_percent: f32,
    /// Memory usage in bytes
    pub memory_bytes: u64,
    /// Process uptime in seconds
    pub uptime_secs: u64,
    /// Process ID (if applicable)
    pub pid: Option<u32>,
}

/// Observable runtime state of a unit
#[derive(Clone, Debug, Default)]
pub struct UnitState {
    pub status: UnitStatus,
    pub health: UnitHealth,
    pub metrics: Option<UnitMetrics>,
}

/// Edge between units (dependencies)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnitEdge {
    pub from: UnitId,
    pub to: UnitId,
    #[serde(default)]
    pub kind: EdgeKind,
}

/// Kind of relationship between units
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    #[default]
    DependsOn,
    TalksTo,
    Produces,
    Consumes,
}
