use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    DesiredState, Edge, EdgeKind, HealthStatus, ObservedState, RuntimeGraph, ServiceId,
    ServiceKind, ServiceNode, ServiceStatus,
};
use crate::unit::{
    EdgeKind as UnitEdgeKind, HealthCheck as UnitHealthCheck, StopBehavior, StopSignal, Unit,
    UnitEdge, UnitKind,
};

/// Health check configuration
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthCheck {
    /// HTTP health check - GET request to path
    Http {
        path: String,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
        #[serde(default = "default_health_timeout")]
        timeout_ms: u64,
    },
    /// TCP port check - just try to connect
    Tcp {
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
    },
    /// Execute a command - exit 0 = healthy
    Exec {
        command: Vec<String>,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
    },
}

fn default_health_interval() -> u64 {
    5000
}
fn default_health_timeout() -> u64 {
    2000
}

/// Restart policy for a service
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Never restart automatically
    Never,
    /// Restart only on non-zero exit
    #[default]
    OnFailure,
    /// Always restart (unless explicitly stopped)
    Always,
}

/// Service definition in the config file
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceConfig {
    /// Display name (defaults to service id)
    #[serde(default)]
    pub name: Option<String>,

    /// Command to run (required for local process engine)
    pub command: Vec<String>,

    /// Working directory for the command
    #[serde(default)]
    pub cwd: Option<PathBuf>,

    /// Environment variables
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Port the service listens on (informational, used for health checks)
    #[serde(default)]
    pub port: Option<u16>,

    /// Service kind (http, worker, database, cache, queue, frontend, generic)
    #[serde(default = "default_kind")]
    pub kind: String,

    /// Auto-start when orkesy launches
    #[serde(default = "default_true")]
    pub autostart: bool,

    /// Health check configuration
    #[serde(default)]
    pub health_check: Option<HealthCheck>,

    /// Services this depends on (affects start order)
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Description for display in UI
    #[serde(default)]
    pub description: Option<String>,

    /// Restart policy
    #[serde(default)]
    pub restart: RestartPolicy,

    /// Delay before restart in milliseconds
    #[serde(default)]
    pub restart_delay_ms: Option<u64>,
}

fn default_kind() -> String {
    "generic".into()
}
fn default_true() -> bool {
    true
}

/// Root configuration file structure
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrkesyConfig {
    /// Config file version
    #[serde(default = "default_version")]
    pub version: String,

    /// Project name
    #[serde(default)]
    pub name: Option<String>,

    /// Service definitions
    pub services: BTreeMap<String, ServiceConfig>,
}

fn default_version() -> String {
    "1".into()
}

/// Configuration loading errors
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    InvalidDependency { service: String, dependency: String },
    MissingCommand { service: String },
    CyclicDependency { cycle: Vec<String> },
    NotFound { searched: Vec<PathBuf> },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Yaml(e) => write!(f, "YAML parse error: {}", e),
            Self::InvalidDependency {
                service,
                dependency,
            } => {
                write!(
                    f,
                    "service '{}' depends on unknown service '{}'",
                    service, dependency
                )
            }
            Self::MissingCommand { service } => {
                write!(f, "service '{}' has no command specified", service)
            }
            Self::CyclicDependency { cycle } => {
                write!(f, "cyclic dependency detected: {}", cycle.join(" -> "))
            }
            Self::NotFound { searched } => {
                write!(f, "no config file found, searched: {:?}", searched)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<serde_yaml::Error> for ConfigError {
    fn from(e: serde_yaml::Error) -> Self {
        ConfigError::Yaml(e)
    }
}

impl OrkesyConfig {
    /// Load configuration from a file
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: OrkesyConfig = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration from a string (useful for testing)
    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        let config: OrkesyConfig = serde_yaml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Search for config file in standard locations
    pub fn discover(start_dir: &Path) -> Result<(PathBuf, Self), ConfigError> {
        let names = ["orkesy.yaml", "orkesy.yml", ".orkesy.yaml", ".orkesy.yml"];
        let mut searched = Vec::new();

        // Check environment variable first
        if let Ok(env_path) = std::env::var("ORKESY_CONFIG") {
            let path = PathBuf::from(&env_path);
            if path.exists() {
                return Ok((path.clone(), Self::load(&path)?));
            }
            searched.push(path);
        }

        // Search current directory and parents
        let mut dir = Some(start_dir);
        while let Some(current) = dir {
            for name in &names {
                let path = current.join(name);
                if path.exists() {
                    return Ok((path.clone(), Self::load(&path)?));
                }
                searched.push(path);
            }
            dir = current.parent();
        }

        Err(ConfigError::NotFound { searched })
    }

    /// Validate the configuration
    fn validate(&self) -> Result<(), ConfigError> {
        // Check all depends_on references exist
        for (id, svc) in &self.services {
            for dep in &svc.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(ConfigError::InvalidDependency {
                        service: id.clone(),
                        dependency: dep.clone(),
                    });
                }
            }

            // Ensure command is not empty
            if svc.command.is_empty() {
                return Err(ConfigError::MissingCommand {
                    service: id.clone(),
                });
            }
        }

        // Check for circular dependencies
        self.check_cycles()?;

        Ok(())
    }

    /// Detect cyclic dependencies using DFS
    fn check_cycles(&self) -> Result<(), ConfigError> {
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Unvisited,
            Visiting,
            Visited,
        }

        let mut states: BTreeMap<&str, State> = self
            .services
            .keys()
            .map(|k| (k.as_str(), State::Unvisited))
            .collect();

        fn dfs<'a>(
            node: &'a str,
            config: &'a OrkesyConfig,
            states: &mut BTreeMap<&'a str, State>,
            path: &mut Vec<&'a str>,
        ) -> Result<(), Vec<String>> {
            states.insert(node, State::Visiting);
            path.push(node);

            if let Some(svc) = config.services.get(node) {
                for dep in &svc.depends_on {
                    match states.get(dep.as_str()) {
                        Some(State::Visiting) => {
                            // Found cycle
                            let cycle_start = path.iter().position(|&n| n == dep.as_str()).unwrap();
                            let mut cycle: Vec<String> =
                                path[cycle_start..].iter().map(|s| s.to_string()).collect();
                            cycle.push(dep.clone());
                            return Err(cycle);
                        }
                        Some(State::Unvisited) | None => {
                            dfs(dep, config, states, path)?;
                        }
                        Some(State::Visited) => {}
                    }
                }
            }

            path.pop();
            states.insert(node, State::Visited);
            Ok(())
        }

        for id in self.services.keys() {
            if states.get(id.as_str()) == Some(&State::Unvisited) {
                let mut path = Vec::new();
                if let Err(cycle) = dfs(id, self, &mut states, &mut path) {
                    return Err(ConfigError::CyclicDependency { cycle });
                }
            }
        }

        Ok(())
    }

    /// Convert to RuntimeGraph
    pub fn to_graph(&self) -> RuntimeGraph {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeSet::new();

        for (id, svc) in &self.services {
            let kind = match svc.kind.to_lowercase().as_str() {
                "http" | "api" | "httpapi" => ServiceKind::HttpApi,
                "worker" => ServiceKind::Worker,
                "database" | "db" => ServiceKind::Database,
                "cache" => ServiceKind::Cache,
                "queue" => ServiceKind::Queue,
                "frontend" => ServiceKind::Frontend,
                _ => ServiceKind::Generic,
            };

            nodes.insert(
                id.clone(),
                ServiceNode {
                    id: id.clone(),
                    display_name: svc.name.clone().unwrap_or_else(|| id.clone()),
                    kind,
                    desired: if svc.autostart {
                        DesiredState::Running
                    } else {
                        DesiredState::Stopped
                    },
                    observed: ObservedState {
                        instance_id: None,
                        status: ServiceStatus::Stopped,
                        health: HealthStatus::Unknown,
                    },
                    port: svc.port,
                    description: svc.description.clone(),
                },
            );

            for dep in &svc.depends_on {
                edges.insert(Edge {
                    from: id.clone(),
                    to: dep.clone(),
                    kind: EdgeKind::DependsOn,
                });
            }
        }

        RuntimeGraph { nodes, edges }
    }

    /// Get services in dependency order (topological sort)
    /// Returns services that have no dependencies first
    pub fn start_order(&self) -> Vec<ServiceId> {
        let mut result = Vec::new();
        let mut visited = BTreeSet::new();

        fn visit(
            id: &str,
            config: &OrkesyConfig,
            visited: &mut BTreeSet<String>,
            result: &mut Vec<String>,
        ) {
            if visited.contains(id) {
                return;
            }
            visited.insert(id.to_string());

            if let Some(svc) = config.services.get(id) {
                for dep in &svc.depends_on {
                    visit(dep, config, visited, result);
                }
            }
            result.push(id.to_string());
        }

        for id in self.services.keys() {
            visit(id, self, &mut visited, &mut result);
        }

        result
    }
}

// ============================================================================
// NEW UNITS-BASED CONFIGURATION (orkesy.yml v1)
// ============================================================================

/// Project metadata in new config format
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProjectConfig {
    pub name: String,
}

/// Unit definition in the new config format
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UnitConfig {
    /// Human-readable display name
    #[serde(default)]
    pub name: Option<String>,

    /// Kind of unit (process, docker, generic)
    #[serde(default)]
    pub kind: Option<String>,

    /// Working directory
    #[serde(default)]
    pub cwd: Option<PathBuf>,

    /// Environment variables
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Install commands
    #[serde(default)]
    pub install: Vec<String>,

    /// Start command (shell string)
    pub start: String,

    /// Stop behavior (signal name or command)
    #[serde(default)]
    pub stop: Option<String>,

    /// Custom logs command (for docker units)
    #[serde(default)]
    pub logs: Option<String>,

    /// Port the unit listens on
    #[serde(default)]
    pub port: Option<u16>,

    /// Health check configuration
    #[serde(default)]
    pub health: Option<UnitHealthConfig>,

    /// Description
    #[serde(default)]
    pub description: Option<String>,

    /// Auto-start when orkesy launches (defaults to false)
    #[serde(default)]
    pub autostart: bool,
}

/// Health check config in new format
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UnitHealthConfig {
    Tcp {
        #[serde(default = "default_health_port")]
        port: u16,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
    },
    Http {
        url: String,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
        #[serde(default = "default_health_timeout")]
        timeout_ms: u64,
    },
    Exec {
        command: String,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
    },
}

fn default_health_port() -> u16 {
    8000
}

/// Edge definition in new config format
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EdgeConfig {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: Option<String>,
}

/// New root config format (orkesy.yml)
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrkesyConfigV2 {
    /// Version (should be 1)
    pub version: u32,

    /// Project metadata
    #[serde(default)]
    pub project: Option<ProjectConfig>,

    /// Unit definitions
    #[serde(default)]
    pub units: BTreeMap<String, UnitConfig>,

    /// Edge definitions
    #[serde(default)]
    pub edges: Vec<EdgeConfig>,
}

impl OrkesyConfigV2 {
    /// Load from file
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: OrkesyConfigV2 = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load from string
    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        let config: OrkesyConfigV2 = serde_yaml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    fn validate(&self) -> Result<(), ConfigError> {
        // Check all edge references exist
        for edge in &self.edges {
            if !self.units.contains_key(&edge.from) {
                return Err(ConfigError::InvalidDependency {
                    service: edge.from.clone(),
                    dependency: "edge 'from' not found".into(),
                });
            }
            if !self.units.contains_key(&edge.to) {
                return Err(ConfigError::InvalidDependency {
                    service: edge.to.clone(),
                    dependency: "edge 'to' not found".into(),
                });
            }
        }

        // Check start commands are not empty
        for (id, unit) in &self.units {
            if unit.start.trim().is_empty() {
                return Err(ConfigError::MissingCommand {
                    service: id.clone(),
                });
            }
        }

        Ok(())
    }

    /// Convert to Vec<Unit> for use with adapters
    pub fn to_units(&self) -> Vec<Unit> {
        self.units
            .iter()
            .map(|(id, cfg)| {
                let kind = match cfg.kind.as_deref() {
                    Some("docker") => UnitKind::Docker,
                    Some("generic") => UnitKind::Generic,
                    _ => UnitKind::Process,
                };

                let stop = cfg
                    .stop
                    .as_ref()
                    .map(|s| match s.to_uppercase().as_str() {
                        "SIGINT" | "INT" => StopBehavior::Signal(StopSignal::SigInt),
                        "SIGTERM" | "TERM" => StopBehavior::Signal(StopSignal::SigTerm),
                        "SIGKILL" | "KILL" => StopBehavior::Signal(StopSignal::SigKill),
                        _ => StopBehavior::Command(s.clone()),
                    })
                    .unwrap_or(StopBehavior::Signal(StopSignal::SigInt));

                let health = cfg.health.as_ref().map(|h| match h {
                    UnitHealthConfig::Tcp { port, interval_ms } => UnitHealthCheck::Tcp {
                        port: *port,
                        interval_ms: *interval_ms,
                    },
                    UnitHealthConfig::Http {
                        url,
                        interval_ms,
                        timeout_ms,
                    } => UnitHealthCheck::Http {
                        url: url.clone(),
                        interval_ms: *interval_ms,
                        timeout_ms: *timeout_ms,
                    },
                    UnitHealthConfig::Exec {
                        command,
                        interval_ms,
                    } => UnitHealthCheck::Exec {
                        command: command.clone(),
                        interval_ms: *interval_ms,
                    },
                });

                Unit {
                    id: id.clone(),
                    name: cfg.name.clone(),
                    kind,
                    cwd: cfg.cwd.clone(),
                    env: cfg.env.clone(),
                    install: cfg.install.clone(),
                    start: cfg.start.clone(),
                    stop,
                    logs: cfg.logs.clone(),
                    health,
                    description: cfg.description.clone(),
                    port: cfg.port,
                    autostart: cfg.autostart,
                }
            })
            .collect()
    }

    /// Convert to Vec<UnitEdge>
    pub fn to_edges(&self) -> Vec<UnitEdge> {
        self.edges
            .iter()
            .map(|e| UnitEdge {
                from: e.from.clone(),
                to: e.to.clone(),
                kind: match e.kind.as_deref() {
                    Some("talks_to") => UnitEdgeKind::TalksTo,
                    Some("produces") => UnitEdgeKind::Produces,
                    Some("consumes") => UnitEdgeKind::Consumes,
                    _ => UnitEdgeKind::DependsOn,
                },
            })
            .collect()
    }

    /// Get project name
    pub fn project_name(&self) -> Option<&str> {
        self.project.as_ref().map(|p| p.name.as_str())
    }
}

/// Try to load config from either old (services) or new (units) format
pub fn load_config(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;

    // Try new format first (has `units:`)
    if content.contains("units:") {
        let config = OrkesyConfigV2::from_str(&content)?;
        return Ok(LoadedConfig::V2(config));
    }

    // Fall back to old format
    let config = OrkesyConfig::from_str(&content)?;
    Ok(LoadedConfig::V1(config))
}

/// Enum to hold either config format
pub enum LoadedConfig {
    V1(OrkesyConfig),
    V2(OrkesyConfigV2),
}

impl LoadedConfig {
    /// Convert to Units (works for both formats)
    pub fn to_units(&self) -> Vec<Unit> {
        match self {
            LoadedConfig::V1(config) => {
                // Convert old ServiceConfig to Unit
                config
                    .services
                    .iter()
                    .map(|(id, svc)| {
                        let kind = match svc.kind.to_lowercase().as_str() {
                            "docker" => UnitKind::Docker,
                            _ => UnitKind::Process,
                        };

                        Unit {
                            id: id.clone(),
                            name: svc.name.clone(),
                            kind,
                            cwd: svc.cwd.clone(),
                            env: svc.env.clone(),
                            install: vec![],
                            start: svc.command.join(" "),
                            stop: StopBehavior::Signal(StopSignal::SigTerm),
                            logs: None,
                            health: svc.health_check.as_ref().map(|h| match h {
                                HealthCheck::Tcp { interval_ms, .. } => UnitHealthCheck::Tcp {
                                    port: svc.port.unwrap_or(8000),
                                    interval_ms: *interval_ms,
                                },
                                HealthCheck::Http {
                                    path,
                                    interval_ms,
                                    timeout_ms,
                                } => UnitHealthCheck::Http {
                                    url: format!(
                                        "http://localhost:{}{}",
                                        svc.port.unwrap_or(8000),
                                        path
                                    ),
                                    interval_ms: *interval_ms,
                                    timeout_ms: *timeout_ms,
                                },
                                HealthCheck::Exec {
                                    command,
                                    interval_ms,
                                } => UnitHealthCheck::Exec {
                                    command: command.join(" "),
                                    interval_ms: *interval_ms,
                                },
                            }),
                            description: svc.description.clone(),
                            port: svc.port,
                            autostart: svc.autostart,
                        }
                    })
                    .collect()
            }
            LoadedConfig::V2(config) => config.to_units(),
        }
    }

    /// Convert to edges
    pub fn to_edges(&self) -> Vec<UnitEdge> {
        match self {
            LoadedConfig::V1(config) => config
                .services
                .iter()
                .flat_map(|(id, svc)| {
                    svc.depends_on.iter().map(move |dep| UnitEdge {
                        from: id.clone(),
                        to: dep.clone(),
                        kind: UnitEdgeKind::DependsOn,
                    })
                })
                .collect(),
            LoadedConfig::V2(config) => config.to_edges(),
        }
    }

    /// Get project name
    pub fn project_name(&self) -> Option<&str> {
        match self {
            LoadedConfig::V1(config) => config.name.as_deref(),
            LoadedConfig::V2(config) => config.project_name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let yaml = r#"
version: "1"
name: test-app
services:
  api:
    command: ["node", "server.js"]
    port: 8000
    kind: http
"#;
        let config = OrkesyConfig::from_str(yaml).unwrap();
        assert_eq!(config.name, Some("test-app".to_string()));
        assert_eq!(config.services.len(), 1);
        assert!(config.services.contains_key("api"));
    }

    #[test]
    fn test_cyclic_dependency_detection() {
        let yaml = r#"
services:
  a:
    command: ["echo"]
    depends_on: [b]
  b:
    command: ["echo"]
    depends_on: [c]
  c:
    command: ["echo"]
    depends_on: [a]
"#;
        let result = OrkesyConfig::from_str(yaml);
        assert!(matches!(result, Err(ConfigError::CyclicDependency { .. })));
    }

    #[test]
    fn test_invalid_dependency() {
        let yaml = r#"
services:
  api:
    command: ["node"]
    depends_on: [nonexistent]
"#;
        let result = OrkesyConfig::from_str(yaml);
        assert!(matches!(result, Err(ConfigError::InvalidDependency { .. })));
    }

    #[test]
    fn test_start_order() {
        let yaml = r#"
services:
  api:
    command: ["node"]
    depends_on: [db]
  db:
    command: ["postgres"]
  worker:
    command: ["python"]
    depends_on: [api, db]
"#;
        let config = OrkesyConfig::from_str(yaml).unwrap();
        let order = config.start_order();

        let db_pos = order.iter().position(|s| s == "db").unwrap();
        let api_pos = order.iter().position(|s| s == "api").unwrap();
        let worker_pos = order.iter().position(|s| s == "worker").unwrap();

        assert!(db_pos < api_pos);
        assert!(db_pos < worker_pos);
        assert!(api_pos < worker_pos);
    }
}
