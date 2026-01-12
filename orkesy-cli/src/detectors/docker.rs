//! Docker Compose project detector
//!
//! Detects docker-compose.yml and provides compose commands.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use orkesy_core::command::{CommandCategory, CommandSpec, DetectedTool};

use super::Detector;

pub struct DockerComposeDetector;

impl DockerComposeDetector {
    /// Find the compose file in the project root
    fn find_compose_file(root: &Path) -> Option<PathBuf> {
        let candidates = [
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ];

        for name in candidates {
            let path = root.join(name);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }
}

#[async_trait]
impl Detector for DockerComposeDetector {
    fn name(&self) -> &'static str {
        "docker-compose"
    }

    async fn detect(&self, root: &Path) -> Option<DetectedTool> {
        Self::find_compose_file(root).map(|file| DetectedTool::DockerCompose { file })
    }

    async fn commands(&self, root: &Path, tool: &DetectedTool) -> Vec<CommandSpec> {
        let DetectedTool::DockerCompose { file } = tool else {
            return vec![];
        };

        let cwd = Some(root.to_path_buf());

        // Read compose file to get service names
        let content = match tokio::fs::read_to_string(file).await {
            Ok(c) => c,
            Err(_) => return Self::base_commands(root, tool),
        };

        let compose: serde_yaml::Value = match serde_yaml::from_str(&content) {
            Ok(v) => v,
            Err(_) => return Self::base_commands(root, tool),
        };

        let mut commands = Self::base_commands(root, tool);

        // Add per-service commands
        if let Some(services) = compose.get("services").and_then(|s| s.as_mapping()) {
            for key in services.keys() {
                if let Some(service_name) = key.as_str() {
                    commands.push(CommandSpec {
                        id: format!("compose:up:{}", service_name),
                        tool: tool.clone(),
                        name: format!("up {}", service_name),
                        display_name: format!("docker compose up {}", service_name),
                        command: format!("docker compose up {}", service_name),
                        cwd: cwd.clone(),
                        description: Some(format!("Start {} service", service_name)),
                        category: CommandCategory::Dev,
                    });

                    commands.push(CommandSpec {
                        id: format!("compose:logs:{}", service_name),
                        tool: tool.clone(),
                        name: format!("logs {}", service_name),
                        display_name: format!("docker compose logs -f {}", service_name),
                        command: format!("docker compose logs -f {}", service_name),
                        cwd: cwd.clone(),
                        description: Some(format!("Follow logs for {}", service_name)),
                        category: CommandCategory::Dev,
                    });
                }
            }
        }

        commands
    }
}

impl DockerComposeDetector {
    fn base_commands(root: &Path, tool: &DetectedTool) -> Vec<CommandSpec> {
        let cwd = Some(root.to_path_buf());

        vec![
            CommandSpec {
                id: "compose:up".into(),
                tool: tool.clone(),
                name: "up".into(),
                display_name: "docker compose up".into(),
                command: "docker compose up".into(),
                cwd: cwd.clone(),
                description: Some("Start all services".into()),
                category: CommandCategory::Dev,
            },
            CommandSpec {
                id: "compose:up-d".into(),
                tool: tool.clone(),
                name: "up -d".into(),
                display_name: "docker compose up -d".into(),
                command: "docker compose up -d".into(),
                cwd: cwd.clone(),
                description: Some("Start all services in background".into()),
                category: CommandCategory::Dev,
            },
            CommandSpec {
                id: "compose:up-build".into(),
                tool: tool.clone(),
                name: "up --build".into(),
                display_name: "docker compose up --build".into(),
                command: "docker compose up --build".into(),
                cwd: cwd.clone(),
                description: Some("Build and start all services".into()),
                category: CommandCategory::Build,
            },
            CommandSpec {
                id: "compose:down".into(),
                tool: tool.clone(),
                name: "down".into(),
                display_name: "docker compose down".into(),
                command: "docker compose down".into(),
                cwd: cwd.clone(),
                description: Some("Stop all services".into()),
                category: CommandCategory::Script,
            },
            CommandSpec {
                id: "compose:logs".into(),
                tool: tool.clone(),
                name: "logs".into(),
                display_name: "docker compose logs -f".into(),
                command: "docker compose logs -f".into(),
                cwd: cwd.clone(),
                description: Some("Follow all logs".into()),
                category: CommandCategory::Dev,
            },
            CommandSpec {
                id: "compose:ps".into(),
                tool: tool.clone(),
                name: "ps".into(),
                display_name: "docker compose ps".into(),
                command: "docker compose ps".into(),
                cwd: cwd.clone(),
                description: Some("List running containers".into()),
                category: CommandCategory::Script,
            },
            CommandSpec {
                id: "compose:pull".into(),
                tool: tool.clone(),
                name: "pull".into(),
                display_name: "docker compose pull".into(),
                command: "docker compose pull".into(),
                cwd: cwd.clone(),
                description: Some("Pull latest images".into()),
                category: CommandCategory::Build,
            },
        ]
    }
}
