//! Commands + Runs feature data models
//!
//! Provides types for detecting project commands and tracking command executions.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Unique identifier for a detected command (e.g., "node:dev", "cargo:build")
pub type CommandId = String;

/// Unique identifier for a command run (UUID)
pub type RunId = String;

/// Package manager for Node.js projects
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    /// Returns the command prefix for running scripts
    pub fn run_prefix(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm run",
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun run",
        }
    }

    /// Returns the install command
    pub fn install_cmd(&self) -> &'static str {
        match self {
            PackageManager::Npm => "npm install",
            PackageManager::Pnpm => "pnpm install",
            PackageManager::Yarn => "yarn install",
            PackageManager::Bun => "bun install",
        }
    }
}

/// Package manager for Python projects
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PythonPackageManager {
    Pip,
    Uv,
    Poetry,
}

/// Tool detected in the project
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetectedTool {
    Node { pm: PackageManager },
    Python { pm: PythonPackageManager },
    Rust,
    Go,
    DockerCompose { file: PathBuf },
    Make { file: PathBuf },
    Just { file: PathBuf },
}

impl DetectedTool {
    /// Short display name for the tool
    pub fn short_name(&self) -> &'static str {
        match self {
            DetectedTool::Node { .. } => "Node",
            DetectedTool::Python { .. } => "Python",
            DetectedTool::Rust => "Rust",
            DetectedTool::Go => "Go",
            DetectedTool::DockerCompose { .. } => "Docker",
            DetectedTool::Make { .. } => "Make",
            DetectedTool::Just { .. } => "Just",
        }
    }

    /// Icon character for the tool
    pub fn icon(&self) -> &'static str {
        match self {
            DetectedTool::Node { .. } => "N",
            DetectedTool::Python { .. } => "P",
            DetectedTool::Rust => "R",
            DetectedTool::Go => "G",
            DetectedTool::DockerCompose { .. } => "D",
            DetectedTool::Make { .. } => "M",
            DetectedTool::Just { .. } => "J",
        }
    }
}

/// Category of a command (affects display grouping and icons)
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CommandCategory {
    Dev,    // Development servers, watch modes
    Build,  // Compilation, bundling
    Test,   // Testing
    Lint,   // Linting, formatting
    Script, // User-defined scripts
    Task,   // Make/Just tasks
}

impl CommandCategory {
    /// Icon for the category
    pub fn icon(&self) -> &'static str {
        match self {
            CommandCategory::Dev => "◉",
            CommandCategory::Build => "⚙",
            CommandCategory::Test => "✓",
            CommandCategory::Lint => "◆",
            CommandCategory::Script => "▸",
            CommandCategory::Task => "≡",
        }
    }

    /// Display label
    pub fn label(&self) -> &'static str {
        match self {
            CommandCategory::Dev => "dev",
            CommandCategory::Build => "build",
            CommandCategory::Test => "test",
            CommandCategory::Lint => "lint",
            CommandCategory::Script => "script",
            CommandCategory::Task => "task",
        }
    }
}

/// A detected command that can be run
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandSpec {
    /// Unique identifier (e.g., "node:dev", "cargo:build")
    pub id: CommandId,
    /// Tool this command belongs to
    pub tool: DetectedTool,
    /// Short name (e.g., "dev", "build")
    pub name: String,
    /// Display name shown in UI (e.g., "pnpm dev")
    pub display_name: String,
    /// Actual shell command to execute
    pub command: String,
    /// Working directory (if different from project root)
    pub cwd: Option<PathBuf>,
    /// Optional description
    pub description: Option<String>,
    /// Category for grouping
    pub category: CommandCategory,
}

/// Status of a command run
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Running,
    Exited { code: Option<i32> },
    Killed,
    Failed { message: String },
}

impl RunStatus {
    /// Icon for the status
    pub fn icon(&self) -> &'static str {
        match self {
            RunStatus::Running => "●",
            RunStatus::Exited { code: Some(0) } => "✓",
            RunStatus::Exited { .. } => "✗",
            RunStatus::Killed => "⊘",
            RunStatus::Failed { .. } => "✗",
        }
    }

    /// Whether the run is still active
    pub fn is_running(&self) -> bool {
        matches!(self, RunStatus::Running)
    }

    /// Whether the run succeeded
    pub fn is_success(&self) -> bool {
        matches!(self, RunStatus::Exited { code: Some(0) })
    }
}

/// A running or completed command execution
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandRun {
    /// Unique run ID (UUID)
    pub id: RunId,
    /// ID of the command that was run
    pub command_id: CommandId,
    /// Full command string
    pub command: String,
    /// Short display name
    pub display_name: String,
    /// Current status
    pub status: RunStatus,
    /// When the run started
    pub started_at: SystemTime,
    /// When the run finished (if finished)
    pub finished_at: Option<SystemTime>,
    /// Exit code (if exited)
    pub exit_code: Option<i32>,
    /// Process ID (if known)
    pub pid: Option<u32>,
}

impl CommandRun {
    /// Duration of the run
    pub fn duration(&self) -> std::time::Duration {
        let end = self.finished_at.unwrap_or_else(SystemTime::now);
        end.duration_since(self.started_at).unwrap_or_default()
    }

    /// Format duration as human-readable string
    pub fn duration_str(&self) -> String {
        let d = self.duration();
        let secs = d.as_secs();
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }
}

/// Indexed project information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectIndex {
    /// Project root directory
    pub root: PathBuf,
    /// Detected tools
    pub tools: Vec<DetectedTool>,
    /// Detected commands indexed by ID
    pub commands: BTreeMap<CommandId, CommandSpec>,
    /// When indexing was performed
    pub indexed_at: SystemTime,
}

impl Default for ProjectIndex {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            tools: Vec::new(),
            commands: BTreeMap::new(),
            indexed_at: SystemTime::UNIX_EPOCH,
        }
    }
}

impl ProjectIndex {
    /// Create a new empty project index
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            tools: Vec::new(),
            commands: BTreeMap::new(),
            indexed_at: SystemTime::now(),
        }
    }

    /// Get commands sorted by category then name
    pub fn commands_sorted(&self) -> Vec<&CommandSpec> {
        let mut cmds: Vec<_> = self.commands.values().collect();
        cmds.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
        cmds
    }
}

// ============================================================================
// Command Registry - Unified command model for Quick Pick
// ============================================================================

/// Unit identifier type alias
pub type UnitId = String;

/// Lifecycle actions for services/units
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleAction {
    Start,
    Stop,
    Restart,
    Toggle,
    Kill,
}

impl LifecycleAction {
    /// Display label for the action
    pub fn label(&self) -> &'static str {
        match self {
            LifecycleAction::Start => "Start",
            LifecycleAction::Stop => "Stop",
            LifecycleAction::Restart => "Restart",
            LifecycleAction::Toggle => "Toggle",
            LifecycleAction::Kill => "Kill",
        }
    }

    /// Icon for the action
    pub fn icon(&self) -> &'static str {
        match self {
            LifecycleAction::Start => "▶",
            LifecycleAction::Stop => "■",
            LifecycleAction::Restart => "⟲",
            LifecycleAction::Toggle => "⇄",
            LifecycleAction::Kill => "✕",
        }
    }
}

/// UI actions (internal operations)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiAction {
    SwitchToLogs,
    SwitchToInspect,
    SwitchToDeps,
    OpenCommandPalette,
    ToggleFocus,
    ClearLogs,
    ToggleFollow,
    Quit,
    Help,
}

impl UiAction {
    /// Display label for the action
    pub fn label(&self) -> &'static str {
        match self {
            UiAction::SwitchToLogs => "Logs View",
            UiAction::SwitchToInspect => "Inspect View",
            UiAction::SwitchToDeps => "Dependencies View",
            UiAction::OpenCommandPalette => "Command Palette",
            UiAction::ToggleFocus => "Toggle Focus",
            UiAction::ClearLogs => "Clear Logs",
            UiAction::ToggleFollow => "Toggle Follow",
            UiAction::Quit => "Quit",
            UiAction::Help => "Help",
        }
    }
}

/// Kind of command in the registry
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandKind {
    /// Service lifecycle action (start/stop/restart/etc)
    Lifecycle {
        unit_id: UnitId,
        action: LifecycleAction,
    },
    /// Execute a named command for a specific unit
    Exec {
        unit_id: UnitId,
        command_id: CommandId,
    },
    /// Project-level command (repo-wide)
    Project { command_id: CommandId },
    /// Internal UI operation
    UI { action: UiAction },
}

impl CommandKind {
    /// Get the unit ID if this command is unit-scoped
    pub fn unit_id(&self) -> Option<&str> {
        match self {
            CommandKind::Lifecycle { unit_id, .. } => Some(unit_id),
            CommandKind::Exec { unit_id, .. } => Some(unit_id),
            CommandKind::Project { .. } => None,
            CommandKind::UI { .. } => None,
        }
    }
}

/// Scope of a command
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandScope {
    /// Global command (available everywhere)
    Global,
    /// Unit-specific command
    Unit(UnitId),
}

/// Source of a command (for plugin attribution)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandSource {
    /// Built-in to Orkesy
    Builtin,
    /// Auto-detected from project files
    Detected,
    /// From orkesy.yml config
    Config,
    /// From a plugin
    Plugin(String),
}

impl CommandSource {
    /// Display label
    pub fn label(&self) -> &str {
        match self {
            CommandSource::Builtin => "builtin",
            CommandSource::Detected => "detected",
            CommandSource::Config => "config",
            CommandSource::Plugin(name) => name,
        }
    }
}

/// Confirmation spec for dangerous actions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfirmSpec {
    /// Message to show in confirmation dialog
    pub message: String,
    /// Whether this action is destructive (affects styling)
    pub destructive: bool,
}

/// Key hint for display (not actual binding)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyHint {
    /// Key or key combination (e.g., "r", "Ctrl+c")
    pub key: String,
    /// Whether this is a global binding
    pub global: bool,
}

/// A command entry in the registry (unified model for Quick Pick)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryCommand {
    /// Unique identifier (namespaced: "builtin.lifecycle.restart.api")
    pub id: CommandId,
    /// Title for Quick Pick display
    pub title: String,
    /// Optional description/detail line
    pub description: Option<String>,
    /// Kind of command
    pub kind: CommandKind,
    /// Tags for filtering (e.g., ["docker", "test", "node"])
    pub tags: Vec<String>,
    /// Scope of the command
    pub scope: CommandScope,
    /// Confirmation required (for dangerous actions)
    pub confirm: Option<ConfirmSpec>,
    /// Default key hint (display-only)
    pub default_key: Option<KeyHint>,
    /// Source of this command
    pub source: CommandSource,
}

impl RegistryCommand {
    /// Create a lifecycle command for a unit
    pub fn lifecycle(unit_id: &str, action: LifecycleAction) -> Self {
        let action_lower = action.label().to_lowercase();
        let needs_confirm = matches!(action, LifecycleAction::Kill);

        Self {
            id: format!("builtin.lifecycle.{}.{}", action_lower, unit_id),
            title: format!("{} {}", action.label(), unit_id),
            description: None,
            kind: CommandKind::Lifecycle {
                unit_id: unit_id.to_string(),
                action: action.clone(),
            },
            tags: vec!["lifecycle".to_string()],
            scope: CommandScope::Unit(unit_id.to_string()),
            confirm: if needs_confirm {
                Some(ConfirmSpec {
                    message: format!("Kill {} and all child processes?", unit_id),
                    destructive: true,
                })
            } else {
                None
            },
            default_key: match action {
                LifecycleAction::Start => Some(KeyHint {
                    key: "t".to_string(),
                    global: false,
                }),
                LifecycleAction::Stop => Some(KeyHint {
                    key: "s".to_string(),
                    global: false,
                }),
                LifecycleAction::Restart => Some(KeyHint {
                    key: "r".to_string(),
                    global: false,
                }),
                LifecycleAction::Toggle => Some(KeyHint {
                    key: "Enter".to_string(),
                    global: false,
                }),
                LifecycleAction::Kill => Some(KeyHint {
                    key: "x".to_string(),
                    global: false,
                }),
            },
            source: CommandSource::Builtin,
        }
    }

    /// Create a UI action command
    pub fn ui_action(action: UiAction) -> Self {
        let key = match &action {
            UiAction::SwitchToLogs => Some("l"),
            UiAction::SwitchToInspect => Some("i"),
            UiAction::SwitchToDeps => Some("d"),
            UiAction::OpenCommandPalette => Some(":"),
            UiAction::ToggleFocus => Some("Tab"),
            UiAction::ClearLogs => Some("c"),
            UiAction::ToggleFollow => Some("f"),
            UiAction::Quit => Some("q"),
            UiAction::Help => Some("?"),
        };

        Self {
            id: format!("builtin.ui.{:?}", action).to_lowercase(),
            title: action.label().to_string(),
            description: None,
            kind: CommandKind::UI { action },
            tags: vec!["ui".to_string()],
            scope: CommandScope::Global,
            confirm: None,
            default_key: key.map(|k| KeyHint {
                key: k.to_string(),
                global: true,
            }),
            source: CommandSource::Builtin,
        }
    }

    /// Create a project command from a detected CommandSpec
    pub fn from_command_spec(spec: &CommandSpec, source: CommandSource) -> Self {
        Self {
            id: format!("{}.{}", source.label(), spec.id),
            title: spec.display_name.clone(),
            description: spec.description.clone(),
            kind: CommandKind::Project {
                command_id: spec.id.clone(),
            },
            tags: vec![
                spec.tool.short_name().to_lowercase(),
                spec.category.label().to_string(),
            ],
            scope: CommandScope::Global,
            confirm: None,
            default_key: None,
            source,
        }
    }

    /// Check if command matches a search query (case-insensitive)
    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        let query_parts: Vec<&str> = query.split_whitespace().collect();

        // Check title
        let title_lower = self.title.to_lowercase();

        // All parts must match somewhere
        query_parts.iter().all(|part| {
            title_lower.contains(part)
                || self.tags.iter().any(|t| t.to_lowercase().contains(part))
                || self
                    .description
                    .as_ref()
                    .map(|d| d.to_lowercase().contains(part))
                    .unwrap_or(false)
                || self.id.to_lowercase().contains(part)
        })
    }

    /// Score for sorting search results (higher = better match)
    pub fn match_score(&self, query: &str) -> u32 {
        let query = query.to_lowercase();
        let title_lower = self.title.to_lowercase();

        let mut score = 0u32;

        // Exact title match
        if title_lower == query {
            score += 100;
        }
        // Title starts with query
        else if title_lower.starts_with(&query) {
            score += 50;
        }
        // Title contains query
        else if title_lower.contains(&query) {
            score += 25;
        }

        // Tag match bonus
        if self.tags.iter().any(|t| t.to_lowercase().contains(&query)) {
            score += 10;
        }

        // Shorter titles rank higher for same matches
        score += (100 - title_lower.len().min(100)) as u32 / 10;

        score
    }
}

/// Command Registry - holds all available commands
#[derive(Clone, Debug, Default)]
pub struct CommandRegistry {
    /// All registered commands
    commands: Vec<RegistryCommand>,
}

impl CommandRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    /// Add a command to the registry
    pub fn add(&mut self, cmd: RegistryCommand) {
        // Avoid duplicates by ID
        if !self.commands.iter().any(|c| c.id == cmd.id) {
            self.commands.push(cmd);
        }
    }

    /// Add lifecycle commands for a unit
    pub fn add_unit_lifecycle(&mut self, unit_id: &str) {
        for action in [
            LifecycleAction::Start,
            LifecycleAction::Stop,
            LifecycleAction::Restart,
            LifecycleAction::Toggle,
            LifecycleAction::Kill,
        ] {
            self.add(RegistryCommand::lifecycle(unit_id, action));
        }
    }

    /// Add all UI action commands
    pub fn add_ui_actions(&mut self) {
        for action in [
            UiAction::SwitchToLogs,
            UiAction::SwitchToInspect,
            UiAction::SwitchToDeps,
            UiAction::OpenCommandPalette,
            UiAction::ToggleFocus,
            UiAction::ClearLogs,
            UiAction::ToggleFollow,
            UiAction::Quit,
            UiAction::Help,
        ] {
            self.add(RegistryCommand::ui_action(action));
        }
    }

    /// Add commands from a ProjectIndex
    pub fn add_from_project_index(&mut self, index: &ProjectIndex) {
        for spec in index.commands.values() {
            self.add(RegistryCommand::from_command_spec(
                spec,
                CommandSource::Detected,
            ));
        }
    }

    /// List all commands, optionally filtered by scope
    pub fn list(&self, scope: Option<&CommandScope>) -> Vec<&RegistryCommand> {
        match scope {
            None => self.commands.iter().collect(),
            Some(filter_scope) => {
                self.commands
                    .iter()
                    .filter(|c| {
                        match (filter_scope, &c.scope) {
                            (CommandScope::Global, _) => true, // Global scope sees all
                            (CommandScope::Unit(filter_id), CommandScope::Unit(cmd_id)) => {
                                filter_id == cmd_id
                            }
                            (CommandScope::Unit(_), CommandScope::Global) => true, // Unit scope sees global
                        }
                    })
                    .collect()
            }
        }
    }

    /// Search commands by query string
    pub fn search(&self, query: &str, scope: Option<&CommandScope>) -> Vec<&RegistryCommand> {
        if query.is_empty() {
            return self.list(scope);
        }

        let mut results: Vec<_> = self
            .list(scope)
            .into_iter()
            .filter(|c| c.matches(query))
            .collect();

        // Sort by match score (descending)
        results.sort_by(|a, b| b.match_score(query).cmp(&a.match_score(query)));

        results
    }

    /// Get a command by ID
    pub fn get(&self, id: &str) -> Option<&RegistryCommand> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Get all lifecycle commands for a specific unit
    pub fn lifecycle_commands(&self, unit_id: &str) -> Vec<&RegistryCommand> {
        self.commands
            .iter()
            .filter(|c| {
                matches!(
                    &c.kind,
                    CommandKind::Lifecycle { unit_id: uid, .. } if uid == unit_id
                )
            })
            .collect()
    }

    /// Get all project commands
    pub fn project_commands(&self) -> Vec<&RegistryCommand> {
        self.commands
            .iter()
            .filter(|c| matches!(&c.kind, CommandKind::Project { .. }))
            .collect()
    }

    /// Get all UI commands
    pub fn ui_commands(&self) -> Vec<&RegistryCommand> {
        self.commands
            .iter()
            .filter(|c| matches!(&c.kind, CommandKind::UI { .. }))
            .collect()
    }

    /// Total number of commands
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_run_duration() {
        let run = CommandRun {
            id: "run-1".to_string(),
            command_id: "test".to_string(),
            command: "npm test".to_string(),
            display_name: "Test".to_string(),
            status: RunStatus::Running,
            started_at: SystemTime::now(),
            finished_at: None,
            exit_code: None,
            pid: Some(1234),
        };

        let duration = run.duration();
        assert!(duration.as_secs() < 1);
    }

    #[test]
    fn test_run_status_icons() {
        assert_eq!(RunStatus::Running.icon(), "●");
        assert_eq!(RunStatus::Exited { code: Some(0) }.icon(), "✓");
        assert_eq!(RunStatus::Exited { code: Some(1) }.icon(), "✗");
        assert_eq!(RunStatus::Killed.icon(), "⊘");
    }

    #[test]
    fn test_run_status_predicates() {
        assert!(RunStatus::Running.is_running());
        assert!(!RunStatus::Exited { code: Some(0) }.is_running());

        assert!(RunStatus::Exited { code: Some(0) }.is_success());
        assert!(!RunStatus::Exited { code: Some(1) }.is_success());
        assert!(!RunStatus::Running.is_success());
    }

    #[test]
    fn test_registry_command_lifecycle() {
        let cmd = RegistryCommand::lifecycle("api", LifecycleAction::Start);

        assert_eq!(cmd.title, "Start api");
        assert!(cmd.id.contains("start"));
        assert!(cmd.id.contains("api"));
        assert!(matches!(cmd.scope, CommandScope::Unit(_)));
    }

    #[test]
    fn test_registry_command_ui_action() {
        let cmd = RegistryCommand::ui_action(UiAction::Quit);

        assert_eq!(cmd.title, "Quit");
        assert!(matches!(cmd.scope, CommandScope::Global));
        assert!(cmd.default_key.is_some());
    }

    #[test]
    fn test_registry_command_matches() {
        let cmd = RegistryCommand::lifecycle("api-server", LifecycleAction::Restart);

        assert!(cmd.matches("api"));
        assert!(cmd.matches("restart"));
        assert!(cmd.matches("API"));
        assert!(!cmd.matches("nonexistent"));
    }

    #[test]
    fn test_registry_command_match_score() {
        let cmd = RegistryCommand::lifecycle("api", LifecycleAction::Start);

        let exact_score = cmd.match_score("Start api");
        let partial_score = cmd.match_score("start");
        let no_match_score = cmd.match_score("xyz");

        assert!(exact_score > partial_score);
        assert!(partial_score > no_match_score);
    }

    #[test]
    fn test_command_registry_add() {
        let mut registry = CommandRegistry::new();
        assert!(registry.is_empty());

        registry.add(RegistryCommand::lifecycle("api", LifecycleAction::Start));
        assert_eq!(registry.len(), 1);

        // Adding duplicate should not increase count
        registry.add(RegistryCommand::lifecycle("api", LifecycleAction::Start));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_command_registry_add_unit_lifecycle() {
        let mut registry = CommandRegistry::new();
        registry.add_unit_lifecycle("api");

        assert_eq!(registry.len(), 5); // Start, Stop, Restart, Toggle, Kill
    }

    #[test]
    fn test_command_registry_add_ui_actions() {
        let mut registry = CommandRegistry::new();
        registry.add_ui_actions();

        assert!(!registry.is_empty());
        assert!(!registry.ui_commands().is_empty());
    }

    #[test]
    fn test_command_registry_search() {
        let mut registry = CommandRegistry::new();
        registry.add_unit_lifecycle("api");
        registry.add_unit_lifecycle("worker");

        let results = registry.search("api", None);
        assert!(!results.is_empty());
        assert!(results.iter().all(|c| c.matches("api")));

        let results = registry.search("start", None);
        // Both api and worker have Start commands, but search also matches lifecycle tag
        assert!(results.len() >= 2);
        assert!(results.iter().all(|c| c.matches("start")));
    }

    #[test]
    fn test_command_registry_list_by_scope() {
        let mut registry = CommandRegistry::new();
        registry.add_unit_lifecycle("api");
        registry.add_ui_actions();

        let global_cmds = registry.list(Some(&CommandScope::Global));
        let unit_cmds = registry.list(Some(&CommandScope::Unit("api".to_string())));

        // Unit scope should see both unit-specific and global commands
        assert!(unit_cmds.len() > global_cmds.len() - registry.ui_commands().len());
    }

    #[test]
    fn test_command_registry_get_by_id() {
        let mut registry = CommandRegistry::new();
        registry.add(RegistryCommand::lifecycle("api", LifecycleAction::Start));

        let cmd = registry.get("builtin.lifecycle.start.api");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().title, "Start api");

        let cmd = registry.get("nonexistent");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_command_registry_lifecycle_commands() {
        let mut registry = CommandRegistry::new();
        registry.add_unit_lifecycle("api");
        registry.add_unit_lifecycle("worker");

        let api_cmds = registry.lifecycle_commands("api");
        assert_eq!(api_cmds.len(), 5);
    }

    #[test]
    fn test_package_manager_prefixes() {
        assert_eq!(PackageManager::Npm.run_prefix(), "npm run");
        assert_eq!(PackageManager::Pnpm.run_prefix(), "pnpm");
        assert_eq!(PackageManager::Yarn.run_prefix(), "yarn");
        assert_eq!(PackageManager::Bun.run_prefix(), "bun run");
    }

    #[test]
    fn test_command_category_icons() {
        assert_eq!(CommandCategory::Dev.icon(), "◉");
        assert_eq!(CommandCategory::Build.icon(), "⚙");
        assert_eq!(CommandCategory::Test.icon(), "✓");
    }

    #[test]
    fn test_detected_tool_short_name() {
        assert_eq!(
            DetectedTool::Node {
                pm: PackageManager::Npm
            }
            .short_name(),
            "Node"
        );
        assert_eq!(DetectedTool::Rust.short_name(), "Rust");
        assert_eq!(DetectedTool::Go.short_name(), "Go");
    }

    #[test]
    fn test_project_index_commands_sorted() {
        let mut index = ProjectIndex::new(PathBuf::from("/test"));

        index.commands.insert(
            "test".to_string(),
            CommandSpec {
                id: "test".to_string(),
                tool: DetectedTool::Node {
                    pm: PackageManager::Npm,
                },
                name: "test".to_string(),
                display_name: "npm test".to_string(),
                command: "npm test".to_string(),
                cwd: None,
                description: None,
                category: CommandCategory::Test,
            },
        );

        index.commands.insert(
            "build".to_string(),
            CommandSpec {
                id: "build".to_string(),
                tool: DetectedTool::Node {
                    pm: PackageManager::Npm,
                },
                name: "build".to_string(),
                display_name: "npm build".to_string(),
                command: "npm run build".to_string(),
                cwd: None,
                description: None,
                category: CommandCategory::Build,
            },
        );

        let sorted = index.commands_sorted();
        assert_eq!(sorted.len(), 2);
        // Build comes before Test alphabetically by category
        assert_eq!(sorted[0].category, CommandCategory::Build);
    }
}
