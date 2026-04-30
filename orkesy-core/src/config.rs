use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    DesiredState, Edge, EdgeKind, HealthStatus, ObservedState, RuntimeGraph, ServiceKind,
    ServiceNode, ServiceStatus,
};
use crate::unit::{
    EdgeKind as UnitEdgeKind, HealthCheck as UnitHealthCheck, StopBehavior, StopSignal, Unit,
    UnitEdge, UnitKind,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthCheck {
    Http {
        path: String,
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
        #[serde(default = "default_health_timeout")]
        timeout_ms: u64,
    },
    Tcp {
        #[serde(default = "default_health_interval")]
        interval_ms: u64,
    },
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Never,
    #[default]
    OnFailure,
    Always,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceConfig {
    #[serde(default)]
    pub name: Option<String>,

    pub command: Vec<String>,

    #[serde(default)]
    pub cwd: Option<PathBuf>,

    #[serde(default)]
    pub env: BTreeMap<String, String>,

    #[serde(default)]
    pub port: Option<u16>,

    #[serde(default = "default_kind")]
    pub kind: String,

    #[serde(default = "default_true")]
    pub autostart: bool,

    #[serde(default)]
    pub health_check: Option<HealthCheck>,

    #[serde(default)]
    pub depends_on: Vec<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub restart: RestartPolicy,

    #[serde(default)]
    pub restart_delay_ms: Option<u64>,
}

fn default_kind() -> String {
    "generic".into()
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OrkesyConfig {
    #[serde(default)]
    pub name: Option<String>,

    pub services: BTreeMap<String, ServiceConfig>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    UnknownUnit {
        referrer: String,
        referenced: String,
    },
    // Kept for the legacy `services:` schema's depends_on validation.
    InvalidDependency {
        service: String,
        dependency: String,
    },
    MissingCommand {
        service: String,
    },
    CyclicDependency {
        cycle: Vec<String>,
    },
    NotFound {
        searched: Vec<PathBuf>,
    },
    UnknownSchema {
        hint: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error reading config: {}", e),
            Self::Yaml(e) => write!(f, "YAML parse error: {}", e),
            Self::UnknownUnit {
                referrer,
                referenced,
            } => {
                write!(
                    f,
                    "edge from '{referrer}' references undeclared unit '{referenced}' (add it under `units:` or remove the edge)"
                )
            }
            Self::InvalidDependency {
                service,
                dependency,
            } => {
                write!(
                    f,
                    "unit '{service}' depends on undeclared unit '{dependency}' (add it under `units:` or remove the dependency)"
                )
            }
            Self::MissingCommand { service } => {
                write!(
                    f,
                    "unit '{service}' has no `start` command — every unit must declare how to start"
                )
            }
            Self::CyclicDependency { cycle } => {
                write!(
                    f,
                    "cyclic dependency: {} (edges form a loop; break one of them)",
                    cycle.join(" → ")
                )
            }
            Self::NotFound { searched } => {
                let lines: Vec<String> = searched
                    .iter()
                    .map(|p| format!("  - {}", p.display()))
                    .collect();
                write!(
                    f,
                    "no orkesy.yml found. Searched:\n{}\n(run `orkesy init` to generate one)",
                    lines.join("\n")
                )
            }
            Self::UnknownSchema { hint } => {
                write!(
                    f,
                    "config file has neither `units:` nor `services:` at the top level: {hint}"
                )
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
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: OrkesyConfig = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn parse(content: &str) -> Result<Self, ConfigError> {
        let config: OrkesyConfig = serde_yaml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn discover(start_dir: &Path) -> Result<(PathBuf, Self), ConfigError> {
        let names = ["orkesy.yaml", "orkesy.yml", ".orkesy.yaml", ".orkesy.yml"];
        let mut searched = Vec::new();

        if let Ok(env_path) = std::env::var("ORKESY_CONFIG") {
            let path = PathBuf::from(&env_path);
            if path.exists() {
                return Ok((path.clone(), Self::load(&path)?));
            }
            searched.push(path);
        }

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

    fn validate(&self) -> Result<(), ConfigError> {
        for (id, svc) in &self.services {
            for dep in &svc.depends_on {
                if !self.services.contains_key(dep) {
                    return Err(ConfigError::InvalidDependency {
                        service: id.clone(),
                        dependency: dep.clone(),
                    });
                }
            }

            if svc.command.is_empty() {
                return Err(ConfigError::MissingCommand {
                    service: id.clone(),
                });
            }
        }

        self.check_cycles()?;
        Ok(())
    }

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

    pub fn start_order(&self) -> Vec<String> {
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

    pub fn to_units(&self) -> Vec<Unit> {
        self.services
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
                            url: format!("http://localhost:{}{}", svc.port.unwrap_or(8000), path),
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

    pub fn to_edges(&self) -> Vec<UnitEdge> {
        self.services
            .iter()
            .flat_map(|(id, svc)| {
                svc.depends_on.iter().map(move |dep| UnitEdge {
                    from: id.clone(),
                    to: dep.clone(),
                    kind: UnitEdgeKind::DependsOn,
                })
            })
            .collect()
    }

    pub fn project_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

// `units:` is the canonical schema; `OrkesyConfig` above is the legacy
// `services:` form, kept for backwards compatibility. Both converge on
// `Workspace` before reaching the runtime.

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProjectMeta {
    #[serde(default)]
    pub name: Option<String>,
}

/// Persistent log history. Disabled by default; see `docs/ARCHITECTURE.md`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogHistoryConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_retention_days")]
    pub retention_days: u32,

    #[serde(default)]
    pub max_size_mb: Option<u64>,
}

impl Default for LogHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: default_retention_days(),
            max_size_mb: None,
        }
    }
}

fn default_retention_days() -> u32 {
    7
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UnitsFile {
    #[serde(default)]
    pub version: Option<u32>,

    #[serde(default)]
    pub project: Option<ProjectMeta>,

    #[serde(default)]
    pub units: BTreeMap<String, Unit>,

    #[serde(default)]
    pub edges: Vec<UnitEdge>,

    #[serde(default)]
    pub log_history: LogHistoryConfig,
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    pub project_name: Option<String>,
    pub units: Vec<Unit>,
    pub edges: Vec<UnitEdge>,
    pub legacy: bool,
    pub log_history: LogHistoryConfig,
}

impl UnitsFile {
    pub fn parse(content: &str) -> Result<Self, ConfigError> {
        let mut file: UnitsFile = serde_yaml::from_str(content)?;
        // `Unit::id` is `#[serde(skip)]`; the YAML map key carries identity.
        for (id, unit) in file.units.iter_mut() {
            unit.id = id.clone();
        }
        file.validate()?;
        Ok(file)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (id, unit) in &self.units {
            if unit.start.trim().is_empty() {
                return Err(ConfigError::MissingCommand {
                    service: id.clone(),
                });
            }
        }

        for edge in &self.edges {
            if !self.units.contains_key(&edge.from) {
                return Err(ConfigError::UnknownUnit {
                    referrer: edge.from.clone(),
                    referenced: edge.from.clone(),
                });
            }
            if !self.units.contains_key(&edge.to) {
                return Err(ConfigError::UnknownUnit {
                    referrer: edge.from.clone(),
                    referenced: edge.to.clone(),
                });
            }
        }

        self.check_cycles()
    }

    fn check_cycles(&self) -> Result<(), ConfigError> {
        let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for id in self.units.keys() {
            adj.insert(id.as_str(), Vec::new());
        }
        for edge in &self.edges {
            if matches!(edge.kind, UnitEdgeKind::DependsOn) {
                adj.entry(edge.from.as_str())
                    .or_default()
                    .push(edge.to.as_str());
            }
        }

        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            Visiting,
            Visited,
        }

        let mut marks: BTreeMap<&str, Mark> = self
            .units
            .keys()
            .map(|k| (k.as_str(), Mark::Unvisited))
            .collect();

        fn dfs<'a>(
            node: &'a str,
            adj: &BTreeMap<&'a str, Vec<&'a str>>,
            marks: &mut BTreeMap<&'a str, Mark>,
            path: &mut Vec<&'a str>,
        ) -> Result<(), Vec<String>> {
            marks.insert(node, Mark::Visiting);
            path.push(node);

            if let Some(neighbours) = adj.get(node) {
                for &next in neighbours {
                    match marks.get(next).copied() {
                        Some(Mark::Visiting) => {
                            let start = path.iter().position(|&n| n == next).unwrap_or(0);
                            let mut cycle: Vec<String> =
                                path[start..].iter().map(|s| s.to_string()).collect();
                            cycle.push(next.to_string());
                            return Err(cycle);
                        }
                        Some(Mark::Unvisited) | None => dfs(next, adj, marks, path)?,
                        Some(Mark::Visited) => {}
                    }
                }
            }

            path.pop();
            marks.insert(node, Mark::Visited);
            Ok(())
        }

        for id in self.units.keys() {
            if marks.get(id.as_str()).copied() == Some(Mark::Unvisited) {
                let mut path = Vec::new();
                if let Err(cycle) = dfs(id.as_str(), &adj, &mut marks, &mut path) {
                    return Err(ConfigError::CyclicDependency { cycle });
                }
            }
        }
        Ok(())
    }

    pub fn into_workspace(self) -> Workspace {
        let project_name = self.project.and_then(|p| p.name);
        let units: Vec<Unit> = self.units.into_values().collect();
        Workspace {
            project_name,
            units,
            edges: self.edges,
            legacy: false,
            log_history: self.log_history,
        }
    }
}

pub fn load_workspace(path: &Path) -> Result<Workspace, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    parse_workspace(&content)
}

pub fn parse_workspace(content: &str) -> Result<Workspace, ConfigError> {
    let top: serde_yaml::Value = serde_yaml::from_str(content)?;
    let map = top.as_mapping().ok_or_else(|| ConfigError::UnknownSchema {
        hint: "expected a top-level mapping".to_string(),
    })?;

    let has_units = map.contains_key(serde_yaml::Value::String("units".into()));
    let has_services = map.contains_key(serde_yaml::Value::String("services".into()));

    if has_units {
        let file = UnitsFile::parse(content)?;
        return Ok(file.into_workspace());
    }
    if has_services {
        let legacy = OrkesyConfig::parse(content)?;
        return Ok(Workspace {
            project_name: legacy.name.clone(),
            units: legacy.to_units(),
            edges: legacy.to_edges(),
            legacy: true,
            log_history: LogHistoryConfig::default(),
        });
    }

    Err(ConfigError::UnknownSchema {
        hint: "expected a top-level `units:` map (run `orkesy init` to generate one)".to_string(),
    })
}

pub fn discover_workspace(start_dir: &Path) -> Result<(PathBuf, Workspace), ConfigError> {
    let names = ["orkesy.yml", "orkesy.yaml", ".orkesy.yml", ".orkesy.yaml"];
    let mut searched = Vec::new();

    if let Ok(env_path) = std::env::var("ORKESY_CONFIG") {
        let path = PathBuf::from(&env_path);
        if path.exists() {
            return Ok((path.clone(), load_workspace(&path)?));
        }
        searched.push(path);
    }

    let mut dir = Some(start_dir);
    while let Some(current) = dir {
        for name in &names {
            let path = current.join(name);
            if path.exists() {
                return Ok((path.clone(), load_workspace(&path)?));
            }
            searched.push(path);
        }
        dir = current.parent();
    }

    Err(ConfigError::NotFound { searched })
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_config() {
        let yaml = r#"
name: test-app
services:
  api:
    command: ["node", "server.js"]
    port: 8000
    kind: http
"#;
        let config = OrkesyConfig::parse(yaml).unwrap();
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
        let result = OrkesyConfig::parse(yaml);
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
        let result = OrkesyConfig::parse(yaml);
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
        let config = OrkesyConfig::parse(yaml).unwrap();
        let order = config.start_order();

        let db_pos = order.iter().position(|s| s == "db").unwrap();
        let api_pos = order.iter().position(|s| s == "api").unwrap();
        let worker_pos = order.iter().position(|s| s == "worker").unwrap();

        assert!(db_pos < api_pos);
        assert!(db_pos < worker_pos);
        assert!(api_pos < worker_pos);
    }

    #[test]
    fn parses_canonical_units_schema() {
        let yaml = r#"
version: 1
project:
  name: my-app
units:
  api:
    kind: process
    start: "npm run dev"
    port: 3000
    autostart: true
edges: []
"#;
        let ws = parse_workspace(yaml).unwrap();
        assert_eq!(ws.project_name.as_deref(), Some("my-app"));
        assert_eq!(ws.units.len(), 1);
        assert_eq!(ws.units[0].id, "api");
        assert_eq!(ws.units[0].port, Some(3000));
        assert!(!ws.legacy);
    }

    #[test]
    fn falls_back_to_legacy_services_schema() {
        let yaml = r#"
name: legacy-app
services:
  api:
    command: ["node", "server.js"]
    port: 8000
"#;
        let ws = parse_workspace(yaml).unwrap();
        assert_eq!(ws.project_name.as_deref(), Some("legacy-app"));
        assert_eq!(ws.units.len(), 1);
        assert!(ws.legacy);
    }

    #[test]
    fn rejects_unit_with_empty_start() {
        let yaml = r#"
units:
  api:
    kind: process
    start: ""
"#;
        let err = parse_workspace(yaml).unwrap_err();
        assert!(matches!(err, ConfigError::MissingCommand { .. }));
    }

    #[test]
    fn rejects_edge_to_unknown_unit() {
        let yaml = r#"
units:
  api:
    start: "node"
edges:
  - { from: api, to: db, kind: depends_on }
"#;
        let err = parse_workspace(yaml).unwrap_err();
        match err {
            ConfigError::UnknownUnit { referenced, .. } => assert_eq!(referenced, "db"),
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn detects_units_schema_cycle() {
        let yaml = r#"
units:
  a: { start: "echo a" }
  b: { start: "echo b" }
  c: { start: "echo c" }
edges:
  - { from: a, to: b, kind: depends_on }
  - { from: b, to: c, kind: depends_on }
  - { from: c, to: a, kind: depends_on }
"#;
        let err = parse_workspace(yaml).unwrap_err();
        assert!(matches!(err, ConfigError::CyclicDependency { .. }));
    }

    #[test]
    fn rejects_file_with_neither_schema() {
        let yaml = "version: 1\nproject: { name: foo }\n";
        let err = parse_workspace(yaml).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownSchema { .. }));
    }

    #[test]
    fn error_messages_are_actionable() {
        let yaml = r#"
units:
  api: { start: "node" }
edges:
  - { from: api, to: missing_db, kind: depends_on }
"#;
        let err = parse_workspace(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("api"), "error should mention referrer: {msg}");
        assert!(
            msg.contains("missing_db"),
            "error should mention missing unit: {msg}"
        );
    }
}
