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
}
