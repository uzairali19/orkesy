//! Rust/Cargo project detector
//!
//! Detects Cargo.toml and provides standard cargo commands.

use std::path::Path;

use async_trait::async_trait;
use orkesy_core::command::{CommandCategory, CommandSpec, DetectedTool};

use super::Detector;

pub struct RustDetector;

#[async_trait]
impl Detector for RustDetector {
    fn name(&self) -> &'static str {
        "rust"
    }

    async fn detect(&self, root: &Path) -> Option<DetectedTool> {
        if root.join("Cargo.toml").exists() {
            Some(DetectedTool::Rust)
        } else {
            None
        }
    }

    async fn commands(&self, root: &Path, tool: &DetectedTool) -> Vec<CommandSpec> {
        let DetectedTool::Rust = tool else {
            return vec![];
        };

        let cwd = Some(root.to_path_buf());

        vec![
            CommandSpec {
                id: "cargo:build".into(),
                tool: tool.clone(),
                name: "build".into(),
                display_name: "cargo build".into(),
                command: "cargo build".into(),
                cwd: cwd.clone(),
                description: Some("Build the project".into()),
                category: CommandCategory::Build,
            },
            CommandSpec {
                id: "cargo:build-release".into(),
                tool: tool.clone(),
                name: "build --release".into(),
                display_name: "cargo build --release".into(),
                command: "cargo build --release".into(),
                cwd: cwd.clone(),
                description: Some("Build in release mode".into()),
                category: CommandCategory::Build,
            },
            CommandSpec {
                id: "cargo:run".into(),
                tool: tool.clone(),
                name: "run".into(),
                display_name: "cargo run".into(),
                command: "cargo run".into(),
                cwd: cwd.clone(),
                description: Some("Run the project".into()),
                category: CommandCategory::Dev,
            },
            CommandSpec {
                id: "cargo:test".into(),
                tool: tool.clone(),
                name: "test".into(),
                display_name: "cargo test".into(),
                command: "cargo test".into(),
                cwd: cwd.clone(),
                description: Some("Run tests".into()),
                category: CommandCategory::Test,
            },
            CommandSpec {
                id: "cargo:check".into(),
                tool: tool.clone(),
                name: "check".into(),
                display_name: "cargo check".into(),
                command: "cargo check".into(),
                cwd: cwd.clone(),
                description: Some("Check for errors without building".into()),
                category: CommandCategory::Lint,
            },
            CommandSpec {
                id: "cargo:clippy".into(),
                tool: tool.clone(),
                name: "clippy".into(),
                display_name: "cargo clippy".into(),
                command: "cargo clippy".into(),
                cwd: cwd.clone(),
                description: Some("Run Clippy lints".into()),
                category: CommandCategory::Lint,
            },
            CommandSpec {
                id: "cargo:fmt".into(),
                tool: tool.clone(),
                name: "fmt".into(),
                display_name: "cargo fmt".into(),
                command: "cargo fmt".into(),
                cwd: cwd.clone(),
                description: Some("Format code".into()),
                category: CommandCategory::Lint,
            },
            CommandSpec {
                id: "cargo:doc".into(),
                tool: tool.clone(),
                name: "doc".into(),
                display_name: "cargo doc".into(),
                command: "cargo doc --open".into(),
                cwd: cwd.clone(),
                description: Some("Build and open documentation".into()),
                category: CommandCategory::Build,
            },
        ]
    }
}
