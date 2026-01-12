//! Plugin System Architecture
//!
//! Provides an extensible system for:
//! - Runtime adapters (process, docker, k8s)
//! - Command detection per ecosystem (node, python, rust, go)
//! - Custom plugins adding units, commands, health checks, metrics
//!
//! Built-in plugins are compiled into the binary; dynamic loading can be added later.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::command::{
    CommandCategory, CommandRegistry, CommandSource, CommandSpec, DetectedTool, PackageManager,
    PythonPackageManager, RegistryCommand,
};
use crate::model::RuntimeGraph;
use crate::unit::Unit;

// ============================================================================
// Plugin Trait
// ============================================================================

/// Context provided to plugins during detection
#[derive(Clone, Debug)]
pub struct DetectContext {
    /// Repository/project root directory
    pub repo_root: PathBuf,
    /// Known configuration files found
    pub known_files: Vec<PathBuf>,
    /// Environment information
    pub env_info: EnvironmentInfo,
}

impl DetectContext {
    /// Create a new detection context
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            known_files: Vec::new(),
            env_info: EnvironmentInfo::default(),
        }
    }

    /// Check if a file exists relative to repo root
    pub fn has_file(&self, name: &str) -> bool {
        self.repo_root.join(name).exists()
    }

    /// Check if any of the given files exist
    pub fn has_any_file(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.has_file(n))
    }

    /// Get the path to a file relative to repo root
    pub fn file_path(&self, name: &str) -> PathBuf {
        self.repo_root.join(name)
    }
}

/// Environment information (from doctor checks)
#[derive(Clone, Debug, Default)]
pub struct EnvironmentInfo {
    /// Available executables on PATH
    pub available_commands: Vec<String>,
    /// Docker available
    pub has_docker: bool,
    /// Node.js version if available
    pub node_version: Option<String>,
    /// Python version if available
    pub python_version: Option<String>,
    /// Rust/Cargo version if available
    pub rust_version: Option<String>,
}

/// Result of plugin detection
#[derive(Clone, Debug, Default)]
pub struct DetectResult {
    /// Confidence score (0.0 to 1.0) that this plugin applies
    pub confidence: f32,
    /// Summary strings for UI display
    pub summary: Vec<String>,
    /// Suggested units to create
    pub suggested_units: Vec<Unit>,
    /// Suggested commands to register
    pub suggested_commands: Vec<CommandSpec>,
}

impl DetectResult {
    /// Create an empty result (plugin doesn't apply)
    pub fn none() -> Self {
        Self::default()
    }

    /// Create a result with a given confidence
    pub fn with_confidence(confidence: f32) -> Self {
        Self {
            confidence,
            ..Default::default()
        }
    }

    /// Add a summary line
    pub fn add_summary(&mut self, s: impl Into<String>) {
        self.summary.push(s.into());
    }

    /// Add a suggested command
    pub fn add_command(&mut self, cmd: CommandSpec) {
        self.suggested_commands.push(cmd);
    }

    /// Add a suggested unit
    pub fn add_unit(&mut self, unit: Unit) {
        self.suggested_units.push(unit);
    }
}

/// Context provided during contribution phase
#[derive(Clone, Debug)]
pub struct PluginContext {
    /// Repository root
    pub repo_root: PathBuf,
    /// Plugin ID
    pub plugin_id: String,
}

/// The main plugin trait
pub trait OrkesyPlugin: Send + Sync {
    /// Unique identifier for this plugin
    fn id(&self) -> &'static str;

    /// Display name for the plugin
    fn name(&self) -> &'static str;

    /// Lightweight detection - runs fast, returns confidence
    fn detect(&self, ctx: &DetectContext) -> DetectResult;

    /// Contribute commands and units to the registry
    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        graph: &mut RuntimeGraph,
    );
}

// ============================================================================
// Built-in Plugins
// ============================================================================

/// Node.js plugin - reads package.json scripts
pub struct NodePlugin;

impl OrkesyPlugin for NodePlugin {
    fn id(&self) -> &'static str {
        "node"
    }

    fn name(&self) -> &'static str {
        "Node.js"
    }

    fn detect(&self, ctx: &DetectContext) -> DetectResult {
        if !ctx.has_file("package.json") {
            return DetectResult::none();
        }

        let mut result = DetectResult::with_confidence(0.9);

        // Determine package manager
        let pm = if ctx.has_file("pnpm-lock.yaml") {
            PackageManager::Pnpm
        } else if ctx.has_file("yarn.lock") {
            PackageManager::Yarn
        } else if ctx.has_file("bun.lockb") {
            PackageManager::Bun
        } else {
            PackageManager::Npm
        };

        result.add_summary(format!("Found package.json ({})", pm.run_prefix()));

        // Try to read package.json and extract scripts
        if let Ok(content) = std::fs::read_to_string(ctx.file_path("package.json")) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
                    let script_count = scripts.len();
                    for (name, _cmd) in scripts {
                        let category = categorize_npm_script(name);
                        let cmd = CommandSpec {
                            id: format!("node:{}", name),
                            tool: DetectedTool::Node { pm: pm.clone() },
                            name: name.to_string(),
                            display_name: format!("{} {}", pm.run_prefix(), name),
                            command: format!("{} {}", pm.run_prefix(), name),
                            cwd: None,
                            description: None,
                            category,
                        };
                        result.add_command(cmd);
                    }
                    result.add_summary(format!("Found {} npm scripts", script_count));
                }
            }
        }

        result
    }

    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        _graph: &mut RuntimeGraph,
    ) {
        // Re-run detection and add commands
        let detect_ctx = DetectContext::new(ctx.repo_root.clone());
        let result = self.detect(&detect_ctx);

        for cmd in result.suggested_commands {
            registry.add(RegistryCommand::from_command_spec(
                &cmd,
                CommandSource::Plugin(self.id().to_string()),
            ));
        }
    }
}

/// Categorize an npm script by name
fn categorize_npm_script(name: &str) -> CommandCategory {
    let name_lower = name.to_lowercase();
    if name_lower.contains("dev") || name_lower.contains("start") || name_lower.contains("serve") {
        CommandCategory::Dev
    } else if name_lower.contains("build") || name_lower.contains("compile") {
        CommandCategory::Build
    } else if name_lower.contains("test") || name_lower.contains("spec") {
        CommandCategory::Test
    } else if name_lower.contains("lint")
        || name_lower.contains("format")
        || name_lower.contains("prettier")
    {
        CommandCategory::Lint
    } else {
        CommandCategory::Script
    }
}

/// Rust/Cargo plugin
pub struct RustPlugin;

impl OrkesyPlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn name(&self) -> &'static str {
        "Rust/Cargo"
    }

    fn detect(&self, ctx: &DetectContext) -> DetectResult {
        if !ctx.has_file("Cargo.toml") {
            return DetectResult::none();
        }

        let mut result = DetectResult::with_confidence(0.9);
        result.add_summary("Found Cargo.toml");

        // Add standard cargo commands
        let commands = [
            ("build", "cargo build", CommandCategory::Build),
            (
                "build --release",
                "cargo build --release",
                CommandCategory::Build,
            ),
            ("test", "cargo test", CommandCategory::Test),
            ("run", "cargo run", CommandCategory::Dev),
            ("clippy", "cargo clippy", CommandCategory::Lint),
            ("fmt", "cargo fmt", CommandCategory::Lint),
            ("check", "cargo check", CommandCategory::Build),
        ];

        for (name, cmd, category) in commands {
            result.add_command(CommandSpec {
                id: format!("cargo:{}", name.replace(' ', "-")),
                tool: DetectedTool::Rust,
                name: name.to_string(),
                display_name: cmd.to_string(),
                command: cmd.to_string(),
                cwd: None,
                description: None,
                category,
            });
        }

        result
    }

    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        _graph: &mut RuntimeGraph,
    ) {
        let detect_ctx = DetectContext::new(ctx.repo_root.clone());
        let result = self.detect(&detect_ctx);

        for cmd in result.suggested_commands {
            registry.add(RegistryCommand::from_command_spec(
                &cmd,
                CommandSource::Plugin(self.id().to_string()),
            ));
        }
    }
}

/// Python plugin - reads pyproject.toml
pub struct PythonPlugin;

impl OrkesyPlugin for PythonPlugin {
    fn id(&self) -> &'static str {
        "python"
    }

    fn name(&self) -> &'static str {
        "Python"
    }

    fn detect(&self, ctx: &DetectContext) -> DetectResult {
        let has_pyproject = ctx.has_file("pyproject.toml");
        let has_requirements = ctx.has_file("requirements.txt");
        let has_setup = ctx.has_file("setup.py");

        if !has_pyproject && !has_requirements && !has_setup {
            return DetectResult::none();
        }

        let mut result = DetectResult::with_confidence(0.8);

        // Determine package manager
        let pm = if ctx.has_file("uv.lock") {
            PythonPackageManager::Uv
        } else if has_pyproject {
            // Check for poetry in pyproject.toml
            if let Ok(content) = std::fs::read_to_string(ctx.file_path("pyproject.toml")) {
                if content.contains("[tool.poetry]") {
                    PythonPackageManager::Poetry
                } else {
                    PythonPackageManager::Uv
                }
            } else {
                PythonPackageManager::Pip
            }
        } else {
            PythonPackageManager::Pip
        };

        result.add_summary(format!("Python project detected ({:?})", pm));

        // Add common Python commands based on package manager
        match pm {
            PythonPackageManager::Uv => {
                result.add_command(create_python_cmd(
                    "uv run",
                    "uv run python",
                    CommandCategory::Dev,
                ));
                result.add_command(create_python_cmd(
                    "uv sync",
                    "uv sync",
                    CommandCategory::Build,
                ));
                result.add_command(create_python_cmd(
                    "uv test",
                    "uv run pytest",
                    CommandCategory::Test,
                ));
            }
            PythonPackageManager::Poetry => {
                result.add_command(create_python_cmd(
                    "poetry run",
                    "poetry run python",
                    CommandCategory::Dev,
                ));
                result.add_command(create_python_cmd(
                    "poetry install",
                    "poetry install",
                    CommandCategory::Build,
                ));
                result.add_command(create_python_cmd(
                    "poetry test",
                    "poetry run pytest",
                    CommandCategory::Test,
                ));
            }
            PythonPackageManager::Pip => {
                result.add_command(create_python_cmd(
                    "pip install",
                    "pip install -r requirements.txt",
                    CommandCategory::Build,
                ));
                result.add_command(create_python_cmd("pytest", "pytest", CommandCategory::Test));
            }
        }

        result
    }

    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        _graph: &mut RuntimeGraph,
    ) {
        let detect_ctx = DetectContext::new(ctx.repo_root.clone());
        let result = self.detect(&detect_ctx);

        for cmd in result.suggested_commands {
            registry.add(RegistryCommand::from_command_spec(
                &cmd,
                CommandSource::Plugin(self.id().to_string()),
            ));
        }
    }
}

fn create_python_cmd(name: &str, command: &str, category: CommandCategory) -> CommandSpec {
    CommandSpec {
        id: format!("python:{}", name.replace(' ', "-")),
        tool: DetectedTool::Python {
            pm: PythonPackageManager::Pip,
        },
        name: name.to_string(),
        display_name: command.to_string(),
        command: command.to_string(),
        cwd: None,
        description: None,
        category,
    }
}

/// Docker Compose plugin
pub struct DockerPlugin;

impl OrkesyPlugin for DockerPlugin {
    fn id(&self) -> &'static str {
        "docker"
    }

    fn name(&self) -> &'static str {
        "Docker Compose"
    }

    fn detect(&self, ctx: &DetectContext) -> DetectResult {
        let compose_files = [
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ];

        let compose_file = compose_files.iter().find(|f| ctx.has_file(f));
        let Some(file) = compose_file else {
            return DetectResult::none();
        };

        let mut result = DetectResult::with_confidence(0.95);
        result.add_summary(format!("Found {}", file));

        // Add standard docker compose commands
        let commands = [
            ("up", "docker compose up", CommandCategory::Dev),
            ("up -d", "docker compose up -d", CommandCategory::Dev),
            ("down", "docker compose down", CommandCategory::Script),
            ("build", "docker compose build", CommandCategory::Build),
            ("logs", "docker compose logs -f", CommandCategory::Script),
            ("ps", "docker compose ps", CommandCategory::Script),
            ("restart", "docker compose restart", CommandCategory::Script),
        ];

        for (name, cmd, category) in commands {
            result.add_command(CommandSpec {
                id: format!("docker:{}", name.replace(' ', "-")),
                tool: DetectedTool::DockerCompose {
                    file: ctx.file_path(file),
                },
                name: name.to_string(),
                display_name: cmd.to_string(),
                command: cmd.to_string(),
                cwd: None,
                description: None,
                category,
            });
        }

        result
    }

    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        _graph: &mut RuntimeGraph,
    ) {
        let detect_ctx = DetectContext::new(ctx.repo_root.clone());
        let result = self.detect(&detect_ctx);

        for cmd in result.suggested_commands {
            registry.add(RegistryCommand::from_command_spec(
                &cmd,
                CommandSource::Plugin(self.id().to_string()),
            ));
        }
    }
}

// ============================================================================
// Plugin Manager
// ============================================================================

/// Manages all registered plugins
pub struct PluginManager {
    plugins: Vec<Box<dyn OrkesyPlugin>>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    /// Create a new plugin manager with built-in plugins
    pub fn new() -> Self {
        let plugins: Vec<Box<dyn OrkesyPlugin>> = vec![
            Box::new(NodePlugin),
            Box::new(RustPlugin),
            Box::new(PythonPlugin),
            Box::new(DockerPlugin),
        ];

        Self { plugins }
    }

    /// Run detection across all plugins
    pub fn detect_all(&self, ctx: &DetectContext) -> BTreeMap<String, DetectResult> {
        let mut results = BTreeMap::new();

        for plugin in &self.plugins {
            let result = plugin.detect(ctx);
            if result.confidence > 0.0 {
                results.insert(plugin.id().to_string(), result);
            }
        }

        results
    }

    /// Contribute all detected plugins to the registry
    pub fn contribute_all(
        &self,
        repo_root: &PathBuf,
        registry: &mut CommandRegistry,
        graph: &mut RuntimeGraph,
    ) {
        let detect_ctx = DetectContext::new(repo_root.clone());

        for plugin in &self.plugins {
            let detect_result = plugin.detect(&detect_ctx);

            // Only contribute if plugin detected something
            if detect_result.confidence > 0.0 {
                let ctx = PluginContext {
                    repo_root: repo_root.clone(),
                    plugin_id: plugin.id().to_string(),
                };
                plugin.contribute(&ctx, registry, graph);
            }
        }
    }

    /// Get plugin by ID
    pub fn get(&self, id: &str) -> Option<&dyn OrkesyPlugin> {
        self.plugins
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    /// List all plugin IDs
    pub fn plugin_ids(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.id()).collect()
    }
}
