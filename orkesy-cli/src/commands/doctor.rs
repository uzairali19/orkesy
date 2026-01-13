use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub message: String,
    pub hint: Option<String>,
}

impl Check {
    fn ok(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: message.into(),
            hint: None,
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: message.into(),
            hint: None,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

pub fn run_doctor() -> Result<(), String> {
    println!("Orkesy Doctor\n");
    println!("Checking environment...\n");

    let mut checks: Vec<Check> = Vec::new();
    let mut warnings: Vec<Check> = Vec::new();

    // === Environment Checks ===
    println!("Environment:");

    // Docker
    checks.push(check_docker());

    // Node.js
    checks.push(check_node());

    // Python
    checks.push(check_python());

    // Rust
    checks.push(check_rust());

    // Go
    checks.push(check_go());

    // Print environment checks
    for check in &checks {
        print_check(check);
    }

    println!();

    // === Config Checks ===
    let config_path = find_config();
    if let Some(path) = &config_path {
        println!("Configuration: {}", path.display());
        println!();

        // Try to load and validate config
        if let Some(unit_checks) = check_config(path) {
            println!("Units:");
            for check in unit_checks {
                print_check(&check);
                if !check.passed {
                    warnings.push(check);
                }
            }
            println!();
        }
    } else {
        println!("Configuration: not found");
        println!("  Run `orkesy init` to create one");
        println!();
    }

    // === Summary ===
    let failed: Vec<_> = checks.iter().filter(|c| !c.passed).collect();
    let total_warnings = warnings.len();

    if failed.is_empty() && total_warnings == 0 {
        println!("All checks passed!");
    } else {
        if !failed.is_empty() {
            println!("Issues found:");
            for check in &failed {
                println!("  - {}: {}", check.name, check.message);
                if let Some(hint) = &check.hint {
                    println!("    Hint: {}", hint);
                }
            }
        }
        if total_warnings > 0 {
            println!("\nWarnings: {} issue(s) with units", total_warnings);
        }
    }

    Ok(())
}

fn print_check(check: &Check) {
    let icon = if check.passed { "✓" } else { "✗" };
    let color = if check.passed { "\x1b[32m" } else { "\x1b[31m" };
    let reset = "\x1b[0m";

    println!(
        "  {}{}{} {}: {}",
        color, icon, reset, check.name, check.message
    );

    if let Some(hint) = &check.hint {
        println!("    └─ {}", hint);
    }
}

fn check_docker() -> Check {
    match Command::new("docker").arg("info").output() {
        Ok(output) if output.status.success() => {
            // Try to get version
            if let Ok(ver_output) = Command::new("docker").arg("--version").output() {
                if ver_output.status.success() {
                    let version = String::from_utf8_lossy(&ver_output.stdout);
                    let version = version
                        .trim()
                        .replace("Docker version ", "")
                        .split(',')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    return Check::ok("docker", format!("v{}", version));
                }
            }
            Check::ok("docker", "running")
        }
        Ok(_) => {
            Check::fail("docker", "not running").with_hint("Start Docker Desktop or run `dockerd`")
        }
        Err(_) => {
            Check::fail("docker", "not installed").with_hint("Install from https://docker.com")
        }
    }
}

fn check_node() -> Check {
    match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Check for package managers
            let mut managers = Vec::new();
            if Command::new("npm")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("npm");
            }
            if Command::new("pnpm")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("pnpm");
            }
            if Command::new("yarn")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("yarn");
            }
            if Command::new("bun")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("bun");
            }

            let pm_info = if managers.is_empty() {
                String::new()
            } else {
                format!(" ({})", managers.join(", "))
            };

            Check::ok("node", format!("{}{}", version, pm_info))
        }
        Ok(_) => Check::fail("node", "error running node"),
        Err(_) => Check::fail("node", "not installed")
            .with_hint("Install from https://nodejs.org or use nvm"),
    }
}

fn check_python() -> Check {
    // Try python3 first, then python
    let python_cmd = if Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python3"
    } else if Command::new("python")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        "python"
    } else {
        return Check::fail("python", "not installed").with_hint("Install from https://python.org");
    };

    match Command::new(python_cmd).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace("Python ", "");

            // Check for package managers
            let mut managers = Vec::new();
            if Command::new("uv")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("uv");
            }
            if Command::new("poetry")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("poetry");
            }
            if Command::new("pip")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                managers.push("pip");
            }

            let pm_info = if managers.is_empty() {
                String::new()
            } else {
                format!(" ({})", managers.join(", "))
            };

            Check::ok("python", format!("v{}{}", version, pm_info))
        }
        _ => Check::fail("python", "error running python"),
    }
}

fn check_rust() -> Check {
    match Command::new("rustc").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace("rustc ", "")
                .split(' ')
                .next()
                .unwrap_or("")
                .to_string();
            Check::ok("rust", format!("v{}", version))
        }
        Ok(_) => Check::fail("rust", "error running rustc"),
        Err(_) => Check::fail("rust", "not installed").with_hint("Install from https://rustup.rs"),
    }
}

fn check_go() -> Check {
    match Command::new("go").arg("version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace("go version go", "")
                .split(' ')
                .next()
                .unwrap_or("")
                .to_string();
            Check::ok("go", format!("v{}", version))
        }
        Ok(_) => Check::fail("go", "error running go"),
        Err(_) => Check::fail("go", "not installed").with_hint("Install from https://go.dev"),
    }
}

fn find_config() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;

    for name in &["orkesy.yml", "orkesy.yaml", ".orkesy.yml", ".orkesy.yaml"] {
        let path = cwd.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn check_config(path: &Path) -> Option<Vec<Check>> {
    use orkesy_core::config::OrkesyConfig;

    let config = OrkesyConfig::load(path).ok()?;
    let units = config.to_units();

    if units.is_empty() {
        return Some(vec![Check::ok("units", "no units defined")]);
    }

    let mut checks = Vec::new();

    for unit in &units {
        let mut issues = Vec::new();

        // Check if cwd exists
        if let Some(cwd) = &unit.cwd {
            let full_path = path.parent().unwrap_or(Path::new(".")).join(cwd);
            if !full_path.exists() {
                issues.push(format!("cwd '{}' not found", cwd.display()));
            }
        }

        // Check if port is available
        if let Some(port) = unit.port {
            if !is_port_available(port) {
                issues.push(format!("port {} is in use", port));
            }
        }

        // Check install commands look reasonable
        for cmd in &unit.install {
            if cmd.is_empty() {
                issues.push("empty install command".to_string());
            }
        }

        // Check start command
        if unit.start.is_empty() {
            issues.push("empty start command".to_string());
        }

        if issues.is_empty() {
            let port_info = unit.port.map(|p| format!(" :{}", p)).unwrap_or_default();
            let auto_info = if unit.autostart { " [autostart]" } else { "" };
            checks.push(Check::ok(&unit.id, format!("ok{}{}", port_info, auto_info)));
        } else {
            checks.push(Check::fail(&unit.id, issues.join(", ")));
        }
    }

    Some(checks)
}

fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}
