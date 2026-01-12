//! `orkesy init` command - scans repo and generates orkesy.yml

use std::fs;
use std::path::{Path, PathBuf};

/// Detected unit from scanning the project
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DetectedUnit {
    pub id: String,
    pub name: Option<String>,
    pub kind: String,
    pub cwd: Option<PathBuf>,
    pub start: String,
    pub stop: Option<String>,
    pub port: Option<u16>,
    pub install: Vec<String>,
    pub description: Option<String>,
    pub autostart: bool,
    pub depends_on: Vec<String>,
}

/// Result of scanning a project
#[derive(Debug, Default)]
pub struct ScanResult {
    pub project_name: Option<String>,
    pub units: Vec<DetectedUnit>,
}

/// Run the init command
pub fn run_init(yes: bool) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;

    // Check if config already exists
    let config_names = ["orkesy.yml", "orkesy.yaml", ".orkesy.yml", ".orkesy.yaml"];
    for name in &config_names {
        let path = cwd.join(name);
        if path.exists() {
            if !yes {
                return Err(format!(
                    "Config file {} already exists. Use --yes to overwrite.",
                    path.display()
                ));
            }
            println!("Overwriting existing config: {}", path.display());
        }
    }

    println!("Scanning project...\n");

    let result = scan_project(&cwd)?;

    if result.units.is_empty() {
        println!("No projects detected. Creating minimal config.\n");
    } else {
        println!("Detected {} unit(s):\n", result.units.len());
        for unit in &result.units {
            let port_info = unit
                .port
                .map(|p| format!(" (port {})", p))
                .unwrap_or_default();
            println!("  {} [{}]{}", unit.id, unit.kind, port_info);
            if let Some(desc) = &unit.description {
                println!("    {}", desc);
            }
        }
        println!();
    }

    // Generate YAML
    let yaml = generate_yaml(&result);

    // Write to file
    let output_path = cwd.join("orkesy.yml");
    fs::write(&output_path, &yaml).map_err(|e| format!("Failed to write config: {}", e))?;

    println!("Created: {}\n", output_path.display());
    println!("Next steps:");
    println!("  1. Review and customize orkesy.yml");
    println!("  2. Run `orkesy` to start the TUI");

    Ok(())
}

/// Scan the project directory for known frameworks/tools
fn scan_project(dir: &Path) -> Result<ScanResult, String> {
    let mut result = ScanResult {
        project_name: detect_project_name(dir),
        ..Default::default()
    };

    // Detect Docker Compose
    if let Some(units) = detect_docker_compose(dir) {
        result.units.extend(units);
    }

    // Detect Node.js projects
    if let Some(units) = detect_node(dir) {
        result.units.extend(units);
    }

    // Detect Python projects
    if let Some(units) = detect_python(dir) {
        result.units.extend(units);
    }

    // Detect Rust projects
    if let Some(units) = detect_rust(dir) {
        result.units.extend(units);
    }

    // Detect Go projects
    if let Some(units) = detect_go(dir) {
        result.units.extend(units);
    }

    Ok(result)
}

fn detect_project_name(dir: &Path) -> Option<String> {
    // Try package.json
    let pkg_json = dir.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_json) {
            if let Some(name) = extract_json_string(&content, "name") {
                return Some(name);
            }
        }
    }

    // Try pyproject.toml
    let pyproject = dir.join("pyproject.toml");
    if pyproject.exists() {
        if let Ok(content) = fs::read_to_string(&pyproject) {
            if let Some(name) = extract_toml_project_name(&content) {
                return Some(name);
            }
        }
    }

    // Try Cargo.toml
    let cargo_toml = dir.join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_toml) {
            if let Some(name) = extract_toml_package_name(&content) {
                return Some(name);
            }
        }
    }

    // Fall back to directory name
    dir.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Detect Docker Compose services
fn detect_docker_compose(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let compose_files = [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ];

    for name in &compose_files {
        let path = dir.join(name);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                return parse_docker_compose(&content, name);
            }
        }
    }
    None
}

fn parse_docker_compose(content: &str, filename: &str) -> Option<Vec<DetectedUnit>> {
    let mut units = Vec::new();

    // Simple parsing: look for service names under "services:"
    let mut in_services = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "services:" {
            in_services = true;
            continue;
        }

        if in_services {
            // Check if we're back to root level
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                break;
            }

            // Service name is a line with minimal indent ending in ':'
            let spaces = line.len() - line.trim_start().len();
            if spaces == 2 && trimmed.ends_with(':') && !trimmed.starts_with('#') {
                let service_name = trimmed.trim_end_matches(':').to_string();

                // Infer type from name
                let (kind_hint, port) = infer_docker_service(&service_name);

                units.push(DetectedUnit {
                    id: service_name.clone(),
                    name: None,
                    kind: "docker".to_string(),
                    cwd: None,
                    start: format!("docker compose -f {} up -d {}", filename, service_name),
                    stop: Some(format!(
                        "docker compose -f {} stop {}",
                        filename, service_name
                    )),
                    port,
                    install: vec![],
                    description: Some(format!("Docker Compose service: {}", service_name)),
                    autostart: kind_hint == "infrastructure",
                    depends_on: vec![],
                });
            }
        }
    }

    if units.is_empty() { None } else { Some(units) }
}

fn infer_docker_service(name: &str) -> (&'static str, Option<u16>) {
    let lower = name.to_lowercase();

    if lower.contains("postgres") || lower.contains("pg") {
        ("infrastructure", Some(5432))
    } else if lower.contains("mysql") || lower.contains("mariadb") {
        ("infrastructure", Some(3306))
    } else if lower.contains("redis") {
        ("infrastructure", Some(6379))
    } else if lower.contains("mongo") {
        ("infrastructure", Some(27017))
    } else if lower.contains("rabbit") {
        ("infrastructure", Some(5672))
    } else if lower.contains("kafka") {
        ("infrastructure", Some(9092))
    } else if lower.contains("elastic") {
        ("infrastructure", Some(9200))
    } else if lower.contains("nginx") {
        ("web", Some(80))
    } else {
        ("service", None)
    }
}

/// Detect Node.js projects
fn detect_node(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let pkg_json = dir.join("package.json");
    if !pkg_json.exists() {
        // Check for monorepo apps
        return detect_node_monorepo(dir);
    }

    let content = fs::read_to_string(&pkg_json).ok()?;
    parse_node_package(&content, dir, None)
}

fn detect_node_monorepo(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let mut units = Vec::new();

    // Common monorepo patterns
    let app_dirs = ["apps", "packages", "services"];

    for app_dir in &app_dirs {
        let apps_path = dir.join(app_dir);
        if apps_path.is_dir() {
            if let Ok(entries) = fs::read_dir(&apps_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let pkg_json = path.join("package.json");
                        if pkg_json.exists() {
                            if let Ok(content) = fs::read_to_string(&pkg_json) {
                                let relative = path.strip_prefix(dir).ok()?;
                                if let Some(mut detected) =
                                    parse_node_package(&content, &path, Some(relative))
                                {
                                    units.append(&mut detected);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if units.is_empty() { None } else { Some(units) }
}

fn parse_node_package(
    content: &str,
    _dir: &Path,
    relative_path: Option<&Path>,
) -> Option<Vec<DetectedUnit>> {
    let mut units = Vec::new();

    let name = extract_json_string(content, "name").unwrap_or_else(|| "app".to_string());
    let id = relative_path
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or(&name)
        .to_string();

    // Detect package manager
    let pkg_manager = detect_node_package_manager(
        relative_path
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
            .as_path(),
    );

    // Check for scripts
    let has_dev = content.contains("\"dev\"");
    let has_start = content.contains("\"start\"");

    if has_dev || has_start {
        let script = if has_dev { "dev" } else { "start" };
        let start_cmd = format!("{} run {}", pkg_manager, script);

        // Try to detect port from scripts
        let port = detect_node_port(content);

        // Detect type from dependencies
        let unit_type = detect_node_type(content);

        units.push(DetectedUnit {
            id: id.clone(),
            name: Some(name.clone()),
            kind: "process".to_string(),
            cwd: relative_path.map(|p| p.to_path_buf()),
            start: start_cmd,
            stop: Some("SIGINT".to_string()),
            port,
            install: vec![format!("{} install", pkg_manager)],
            description: Some(format!("{} ({})", unit_type, pkg_manager)),
            autostart: true,
            depends_on: vec![],
        });
    }

    if units.is_empty() { None } else { Some(units) }
}

fn detect_node_package_manager(dir: &Path) -> &'static str {
    if dir.join("pnpm-lock.yaml").exists() || dir.join("pnpm-workspace.yaml").exists() {
        "pnpm"
    } else if dir.join("yarn.lock").exists() {
        "yarn"
    } else if dir.join("bun.lockb").exists() {
        "bun"
    } else {
        "npm"
    }
}

fn detect_node_port(content: &str) -> Option<u16> {
    // Look for common port patterns
    if content.contains("vite") || content.contains("5173") {
        Some(5173)
    } else if content.contains("next") || content.contains("3000") {
        Some(3000)
    } else if content.contains("\"port\": 8080") || content.contains("8080") {
        Some(8080)
    } else if content.contains("\"port\": 3000") {
        Some(3000)
    } else {
        None
    }
}

fn detect_node_type(content: &str) -> &'static str {
    if content.contains("\"next\"") {
        "Next.js app"
    } else if content.contains("\"vite\"") || content.contains("\"@vitejs") {
        "Vite app"
    } else if content.contains("\"react\"") {
        "React app"
    } else if content.contains("\"vue\"") {
        "Vue app"
    } else if content.contains("\"express\"") {
        "Express server"
    } else if content.contains("\"fastify\"") {
        "Fastify server"
    } else if content.contains("\"nest\"") || content.contains("\"@nestjs") {
        "NestJS server"
    } else {
        "Node.js app"
    }
}

/// Detect Python projects
fn detect_python(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let mut units = Vec::new();

    // Check for pyproject.toml (modern Python)
    let pyproject = dir.join("pyproject.toml");
    if pyproject.exists() {
        if let Ok(content) = fs::read_to_string(&pyproject) {
            if let Some(unit) = parse_python_project(&content, dir) {
                units.push(unit);
            }
        }
    }

    // Check for requirements.txt (traditional Python)
    let requirements = dir.join("requirements.txt");
    if requirements.exists() && units.is_empty() {
        if let Ok(content) = fs::read_to_string(&requirements) {
            if let Some(unit) = parse_python_requirements(&content, dir) {
                units.push(unit);
            }
        }
    }

    if units.is_empty() { None } else { Some(units) }
}

fn parse_python_project(content: &str, dir: &Path) -> Option<DetectedUnit> {
    let name = extract_toml_project_name(content).unwrap_or_else(|| "api".to_string());

    // Detect if using uv, poetry, or pip
    let pkg_manager = if dir.join("uv.lock").exists() {
        "uv"
    } else if content.contains("[tool.poetry]") {
        "poetry"
    } else {
        "pip"
    };

    // Detect framework
    let (framework, start_cmd, port) = if content.contains("fastapi") || content.contains("FastAPI")
    {
        (
            "FastAPI",
            format!("{} run uvicorn main:app --reload", pkg_manager),
            Some(8000),
        )
    } else if content.contains("django") || content.contains("Django") {
        (
            "Django",
            format!("{} run python manage.py runserver", pkg_manager),
            Some(8000),
        )
    } else if content.contains("flask") || content.contains("Flask") {
        (
            "Flask",
            format!("{} run flask run", pkg_manager),
            Some(5000),
        )
    } else if content.contains("uvicorn") {
        (
            "ASGI app",
            format!("{} run uvicorn main:app --reload", pkg_manager),
            Some(8000),
        )
    } else {
        (
            "Python app",
            format!("{} run python main.py", pkg_manager),
            None,
        )
    };

    let install_cmd = match pkg_manager {
        "uv" => "uv sync".to_string(),
        "poetry" => "poetry install".to_string(),
        _ => "pip install -r requirements.txt".to_string(),
    };

    Some(DetectedUnit {
        id: name.clone(),
        name: Some(name),
        kind: "process".to_string(),
        cwd: None,
        start: start_cmd,
        stop: Some("SIGINT".to_string()),
        port,
        install: vec![install_cmd],
        description: Some(format!("{} ({})", framework, pkg_manager)),
        autostart: true,
        depends_on: vec![],
    })
}

fn parse_python_requirements(content: &str, _dir: &Path) -> Option<DetectedUnit> {
    let (framework, start_cmd, port) = if content.contains("fastapi") || content.contains("uvicorn")
    {
        ("FastAPI", "uvicorn main:app --reload", Some(8000))
    } else if content.contains("django") {
        ("Django", "python manage.py runserver", Some(8000))
    } else if content.contains("flask") {
        ("Flask", "flask run", Some(5000))
    } else {
        return None;
    };

    Some(DetectedUnit {
        id: "api".to_string(),
        name: Some(framework.to_string()),
        kind: "process".to_string(),
        cwd: None,
        start: start_cmd.to_string(),
        stop: Some("SIGINT".to_string()),
        port,
        install: vec!["pip install -r requirements.txt".to_string()],
        description: Some(format!("{} server", framework)),
        autostart: true,
        depends_on: vec![],
    })
}

/// Detect Rust projects
fn detect_rust(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let cargo_toml = dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return None;
    }

    let content = fs::read_to_string(&cargo_toml).ok()?;
    let name = extract_toml_package_name(&content).unwrap_or_else(|| "app".to_string());

    // Check if it's a binary or library
    let is_bin = content.contains("[[bin]]")
        || dir.join("src/main.rs").exists()
        || !dir.join("src/lib.rs").exists();

    if !is_bin {
        return None;
    }

    // Detect framework
    let (framework, port) = if content.contains("actix") {
        ("Actix Web", Some(8080))
    } else if content.contains("axum") {
        ("Axum", Some(3000))
    } else if content.contains("rocket") {
        ("Rocket", Some(8000))
    } else if content.contains("warp") {
        ("Warp", Some(3030))
    } else {
        ("Rust binary", None)
    };

    Some(vec![DetectedUnit {
        id: name.clone(),
        name: Some(name),
        kind: "process".to_string(),
        cwd: None,
        start: "cargo run --release".to_string(),
        stop: Some("SIGINT".to_string()),
        port,
        install: vec!["cargo build --release".to_string()],
        description: Some(framework.to_string()),
        autostart: port.is_some(), // Only autostart if it's a server
        depends_on: vec![],
    }])
}

/// Detect Go projects
fn detect_go(dir: &Path) -> Option<Vec<DetectedUnit>> {
    let go_mod = dir.join("go.mod");
    if !go_mod.exists() {
        return None;
    }

    let content = fs::read_to_string(&go_mod).ok()?;

    // Extract module name
    let name = content
        .lines()
        .find(|l| l.starts_with("module "))
        .and_then(|l| l.strip_prefix("module "))
        .map(|s| s.split('/').next_back().unwrap_or(s).to_string())
        .unwrap_or_else(|| "app".to_string());

    // Check for main.go
    let has_main = dir.join("main.go").exists() || dir.join("cmd").exists();
    if !has_main {
        return None;
    }

    // Detect framework from go.sum or imports
    let go_sum = dir.join("go.sum");
    let deps = fs::read_to_string(&go_sum).unwrap_or_default();

    let (framework, port) = if deps.contains("gin-gonic") {
        ("Gin", Some(8080))
    } else if deps.contains("labstack/echo") {
        ("Echo", Some(8080))
    } else if deps.contains("gofiber/fiber") {
        ("Fiber", Some(3000))
    } else if deps.contains("go-chi/chi") {
        ("Chi", Some(8080))
    } else {
        ("Go app", None)
    };

    Some(vec![DetectedUnit {
        id: name.clone(),
        name: Some(name),
        kind: "process".to_string(),
        cwd: None,
        start: "go run .".to_string(),
        stop: Some("SIGINT".to_string()),
        port,
        install: vec!["go mod download".to_string()],
        description: Some(framework.to_string()),
        autostart: port.is_some(),
        depends_on: vec![],
    }])
}

// Helper functions for parsing

fn extract_json_string(content: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let idx = content.find(&pattern)?;
    let rest = &content[idx + pattern.len()..];
    let rest = rest.trim_start().strip_prefix(':')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_toml_project_name(content: &str) -> Option<String> {
    // Look for [project] section name or [tool.poetry] name
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") && trimmed.contains('=') {
            let value = trimmed.split('=').nth(1)?.trim();
            let value = value.trim_matches('"').trim_matches('\'');
            return Some(value.to_string());
        }
    }
    None
}

fn extract_toml_package_name(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') && in_package {
            break;
        }
        if in_package && trimmed.starts_with("name") {
            let value = trimmed.split('=').nth(1)?.trim();
            let value = value.trim_matches('"').trim_matches('\'');
            return Some(value.to_string());
        }
    }
    None
}

/// Generate YAML configuration from scan results
fn generate_yaml(result: &ScanResult) -> String {
    let mut yaml = String::new();

    yaml.push_str("# Orkesy Configuration\n");
    yaml.push_str("# Generated by `orkesy init`\n\n");
    yaml.push_str("version: 1\n\n");

    // Project section
    yaml.push_str("project:\n");
    let name = result.project_name.as_deref().unwrap_or("my-app");
    yaml.push_str(&format!("  name: {}\n\n", name));

    // Units section
    yaml.push_str("units:\n");

    if result.units.is_empty() {
        yaml.push_str("  # Add your units here\n");
        yaml.push_str("  # example:\n");
        yaml.push_str("  #   kind: process\n");
        yaml.push_str("  #   start: \"npm run dev\"\n");
        yaml.push_str("  #   port: 3000\n");
    } else {
        for unit in &result.units {
            yaml.push_str(&format!("  {}:\n", unit.id));
            yaml.push_str(&format!("    kind: {}\n", unit.kind));

            if let Some(cwd) = &unit.cwd {
                yaml.push_str(&format!("    cwd: ./{}\n", cwd.display()));
            }

            if !unit.install.is_empty() {
                yaml.push_str("    install:\n");
                for cmd in &unit.install {
                    yaml.push_str(&format!("      - \"{}\"\n", cmd));
                }
            }

            yaml.push_str(&format!("    start: \"{}\"\n", unit.start));

            if let Some(stop) = &unit.stop {
                yaml.push_str(&format!("    stop: {}\n", stop));
            }

            if let Some(port) = unit.port {
                yaml.push_str(&format!("    port: {}\n", port));
            }

            yaml.push_str(&format!("    autostart: {}\n", unit.autostart));

            if let Some(desc) = &unit.description {
                yaml.push_str(&format!("    description: \"{}\"\n", desc));
            }

            yaml.push('\n');
        }
    }

    // Edges section
    yaml.push_str("# Dependencies between units\n");
    yaml.push_str("edges: []\n");
    yaml.push_str("  # - from: api\n");
    yaml.push_str("  #   to: db\n");
    yaml.push_str("  #   kind: depends_on\n");

    yaml
}
