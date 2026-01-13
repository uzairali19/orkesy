use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type UnitId = String;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    #[default]
    Process,
    Docker,
    Generic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopBehavior {
    Signal(StopSignal),
    Command(String),
}

impl Default for StopBehavior {
    fn default() -> Self {
        StopBehavior::Signal(StopSignal::SigInt)
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HealthCheck {
    Tcp {
        #[serde(default = "default_health_port")]
        port: u16,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
    },
    Http {
        url: String,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
        #[serde(default = "default_timeout_ms")]
        timeout_ms: u64,
    },
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
    #[serde(skip)]
    pub id: UnitId,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub kind: UnitKind,

    #[serde(default)]
    pub cwd: Option<PathBuf>,

    #[serde(default)]
    pub env: BTreeMap<String, String>,

    #[serde(default)]
    pub install: Vec<String>,

    pub start: String,

    #[serde(default)]
    pub stop: StopBehavior,

    #[serde(default)]
    pub logs: Option<String>,

    #[serde(default)]
    pub health: Option<HealthCheck>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub port: Option<u16>,

    #[serde(default = "default_autostart")]
    pub autostart: bool,
}

fn default_autostart() -> bool {
    false
}

impl Unit {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

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

#[derive(Clone, Debug, Default)]
pub struct UnitMetrics {
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub uptime_secs: u64,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct UnitState {
    pub status: UnitStatus,
    pub health: UnitHealth,
    pub metrics: Option<UnitMetrics>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UnitEdge {
    pub from: UnitId,
    pub to: UnitId,
    #[serde(default)]
    pub kind: EdgeKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    #[default]
    DependsOn,
    TalksTo,
    Produces,
    Consumes,
}
