use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::command::{
    CommandCategory, CommandRegistry, CommandSource, CommandSpec, DetectedTool, PackageManager,
    PythonPackageManager, RegistryCommand,
};
use crate::model::RuntimeGraph;
use crate::unit::Unit;

#[derive(Clone, Debug)]
pub struct DetectContext {
    pub repo_root: PathBuf,
    pub known_files: Vec<PathBuf>,
    pub env_info: EnvironmentInfo,
}

impl DetectContext {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            known_files: Vec::new(),
            env_info: EnvironmentInfo::default(),
        }
    }

    pub fn has_file(&self, name: &str) -> bool {
        self.repo_root.join(name).exists()
    }

    pub fn has_any_file(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.has_file(n))
    }

    pub fn file_path(&self, name: &str) -> PathBuf {
        self.repo_root.join(name)
    }
}

#[derive(Clone, Debug, Default)]
pub struct EnvironmentInfo {
    pub available_commands: Vec<String>,
    pub has_docker: bool,
    pub node_version: Option<String>,
    pub python_version: Option<String>,
    pub rust_version: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DetectResult {
    pub confidence: f32,
    pub summary: Vec<String>,
    pub suggested_units: Vec<Unit>,
    pub suggested_commands: Vec<CommandSpec>,
}

impl DetectResult {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_confidence(confidence: f32) -> Self {
        Self {
            confidence,
            ..Default::default()
        }
    }

    pub fn add_summary(&mut self, s: impl Into<String>) {
        self.summary.push(s.into());
    }

    pub fn add_command(&mut self, cmd: CommandSpec) {
        self.suggested_commands.push(cmd);
    }

    pub fn add_unit(&mut self, unit: Unit) {
        self.suggested_units.push(unit);
    }
}

#[derive(Clone, Debug)]
pub struct PluginContext {
    pub repo_root: PathBuf,
    pub plugin_id: String,
}

pub trait OrkesyPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn detect(&self, ctx: &DetectContext) -> DetectResult;
    fn contribute(
        &self,
        ctx: &PluginContext,
        registry: &mut CommandRegistry,
        graph: &mut RuntimeGraph,
    );
}

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

        if let Ok(content) = std::fs::read_to_string(ctx.file_path("package.json"))
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(scripts) = json.get("scripts").and_then(|s| s.as_object())
        {
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

        let pm = if ctx.has_file("uv.lock") {
            PythonPackageManager::Uv
        } else if has_pyproject {
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

pub struct PluginManager {
    plugins: Vec<Box<dyn OrkesyPlugin>>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    pub fn new() -> Self {
        let plugins: Vec<Box<dyn OrkesyPlugin>> = vec![
            Box::new(NodePlugin),
            Box::new(RustPlugin),
            Box::new(PythonPlugin),
            Box::new(DockerPlugin),
        ];

        Self { plugins }
    }

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

    pub fn contribute_all(
        &self,
        repo_root: &Path,
        registry: &mut CommandRegistry,
        graph: &mut RuntimeGraph,
    ) {
        let detect_ctx = DetectContext::new(repo_root.to_path_buf());

        for plugin in &self.plugins {
            let detect_result = plugin.detect(&detect_ctx);

            if detect_result.confidence > 0.0 {
                let ctx = PluginContext {
                    repo_root: repo_root.to_path_buf(),
                    plugin_id: plugin.id().to_string(),
                };
                plugin.contribute(&ctx, registry, graph);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&dyn OrkesyPlugin> {
        self.plugins
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    pub fn plugin_ids(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.id()).collect()
    }
}
