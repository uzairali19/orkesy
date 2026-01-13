use std::path::Path;

use async_trait::async_trait;
use orkesy_core::command::{CommandCategory, CommandSpec, DetectedTool, PackageManager};

use super::Detector;

pub struct NodeDetector;

impl NodeDetector {
    fn detect_package_manager(root: &Path) -> PackageManager {
        if root.join("pnpm-lock.yaml").exists() {
            PackageManager::Pnpm
        } else if root.join("yarn.lock").exists() {
            PackageManager::Yarn
        } else if root.join("bun.lockb").exists() {
            PackageManager::Bun
        } else {
            PackageManager::Npm
        }
    }

    fn categorize_script(name: &str) -> CommandCategory {
        let lower = name.to_lowercase();
        if lower == "dev"
            || lower == "start"
            || lower == "serve"
            || lower.starts_with("dev:")
            || lower.contains("watch")
        {
            CommandCategory::Dev
        } else if lower == "build" || lower.starts_with("build:") || lower == "compile" {
            CommandCategory::Build
        } else if lower == "test" || lower.starts_with("test:") || lower == "coverage" {
            CommandCategory::Test
        } else if lower == "lint"
            || lower == "format"
            || lower == "check"
            || lower.starts_with("lint:")
            || lower == "prettier"
            || lower == "eslint"
        {
            CommandCategory::Lint
        } else {
            CommandCategory::Script
        }
    }
}

#[async_trait]
impl Detector for NodeDetector {
    fn name(&self) -> &'static str {
        "node"
    }

    async fn detect(&self, root: &Path) -> Option<DetectedTool> {
        let pkg_json = root.join("package.json");
        if pkg_json.exists() {
            Some(DetectedTool::Node {
                pm: Self::detect_package_manager(root),
            })
        } else {
            None
        }
    }

    async fn commands(&self, root: &Path, tool: &DetectedTool) -> Vec<CommandSpec> {
        let DetectedTool::Node { pm } = tool else {
            return vec![];
        };

        let pkg_json = root.join("package.json");
        let content = match tokio::fs::read_to_string(&pkg_json).await {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let pkg: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return vec![],
        };

        let mut commands = Vec::new();

        // Add install command
        commands.push(CommandSpec {
            id: "node:install".into(),
            tool: tool.clone(),
            name: "install".into(),
            display_name: pm.install_cmd().into(),
            command: pm.install_cmd().into(),
            cwd: Some(root.to_path_buf()),
            description: Some("Install dependencies".into()),
            category: CommandCategory::Build,
        });

        // Extract scripts from package.json
        if let Some(scripts) = pkg.get("scripts").and_then(|s| s.as_object()) {
            let prefix = pm.run_prefix();

            for (name, _cmd) in scripts {
                let category = Self::categorize_script(name);
                let display = format!("{} {}", prefix, name);
                let cmd_str = format!("{} {}", prefix, name);

                commands.push(CommandSpec {
                    id: format!("node:{}", name),
                    tool: tool.clone(),
                    name: name.clone(),
                    display_name: display,
                    command: cmd_str,
                    cwd: Some(root.to_path_buf()),
                    description: None,
                    category,
                });
            }
        }

        commands
    }
}
