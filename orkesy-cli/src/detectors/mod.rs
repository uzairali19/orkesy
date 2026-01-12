//! Project detection framework
//!
//! Detects available tools and commands in a project directory.

mod docker;
mod node;
mod rust;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use async_trait::async_trait;
use orkesy_core::command::{CommandSpec, DetectedTool, ProjectIndex};

pub use docker::DockerComposeDetector;
pub use node::NodeDetector;
pub use rust::RustDetector;

/// Trait for project detectors
#[async_trait]
pub trait Detector: Send + Sync {
    /// Name of this detector
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    /// Check if this detector applies to the project
    async fn detect(&self, root: &Path) -> Option<DetectedTool>;

    /// Extract commands from the project
    async fn commands(&self, root: &Path, tool: &DetectedTool) -> Vec<CommandSpec>;
}

/// Run all detectors and build a ProjectIndex
pub async fn index_project(root: &Path) -> ProjectIndex {
    let detectors: Vec<Box<dyn Detector>> = vec![
        Box::new(NodeDetector),
        Box::new(RustDetector),
        Box::new(DockerComposeDetector),
    ];

    let mut tools = Vec::new();
    let mut commands = BTreeMap::new();

    for detector in &detectors {
        if let Some(tool) = detector.detect(root).await {
            let cmds = detector.commands(root, &tool).await;
            for cmd in cmds {
                commands.insert(cmd.id.clone(), cmd);
            }
            tools.push(tool);
        }
    }

    ProjectIndex {
        root: root.to_path_buf(),
        tools,
        commands,
        indexed_at: SystemTime::now(),
    }
}
