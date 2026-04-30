use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orkesy_core::config::{OrkesyConfig, discover_workspace};
use orkesy_core::unit::{EdgeKind as UnitEdgeKind, Unit, UnitEdge, UnitKind};

pub fn run_migrate(yes: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let (path, ws) = discover_workspace(&cwd).map_err(|e| format!("{e}"))?;

    if !ws.legacy {
        println!(
            "{}: already uses the canonical `units:` schema. Nothing to do.",
            path.display()
        );
        return Ok(());
    }

    let original =
        fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let legacy =
        OrkesyConfig::parse(&original).map_err(|e| format!("re-parsing legacy config: {e}"))?;

    let units = legacy.to_units();
    let edges = legacy.to_edges();
    let yaml = generate_units_yaml(legacy.project_name(), &units, &edges);

    if !yes {
        println!(
            "Will rewrite {} (backup will be saved alongside).\n\
             Re-run with --yes to confirm.",
            path.display()
        );
        return Ok(());
    }

    let backup = backup_path(&path);
    fs::copy(&path, &backup).map_err(|e| format!("writing backup {}: {e}", backup.display()))?;
    fs::write(&path, &yaml).map_err(|e| format!("writing {}: {e}", path.display()))?;

    println!("Migrated: {}", path.display());
    println!("Backup:   {}", backup.display());
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut p = path.to_path_buf();
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("orkesy.yml");
    p.set_file_name(format!("{file}.{ts}.bak"));
    p
}

fn generate_units_yaml(project: Option<&str>, units: &[Unit], edges: &[UnitEdge]) -> String {
    let mut out = String::new();
    out.push_str("# Orkesy configuration (migrated from legacy `services:` schema)\n");
    out.push_str("version: 1\n\n");

    if let Some(name) = project {
        out.push_str("project:\n");
        out.push_str(&format!("  name: {name}\n\n"));
    }

    out.push_str("units:\n");
    for unit in units {
        out.push_str(&format!("  {}:\n", unit.id));
        out.push_str(&format!("    kind: {}\n", kind_label(&unit.kind)));
        if let Some(cwd) = &unit.cwd {
            out.push_str(&format!("    cwd: {}\n", cwd.display()));
        }
        out.push_str(&format!("    start: {}\n", yaml_string(&unit.start)));
        if let Some(port) = unit.port {
            out.push_str(&format!("    port: {port}\n"));
        }
        if !unit.env.is_empty() {
            out.push_str("    env:\n");
            for (k, v) in &unit.env {
                out.push_str(&format!("      {k}: {}\n", yaml_string(v)));
            }
        }
        out.push_str(&format!("    autostart: {}\n", unit.autostart));
        if let Some(desc) = &unit.description {
            out.push_str(&format!("    description: {}\n", yaml_string(desc)));
        }
        out.push('\n');
    }

    if edges.is_empty() {
        out.push_str("edges: []\n");
    } else {
        out.push_str("edges:\n");
        for edge in edges {
            out.push_str(&format!(
                "  - {{ from: {}, to: {}, kind: {} }}\n",
                edge.from,
                edge.to,
                edge_kind_label(&edge.kind)
            ));
        }
    }

    out
}

fn kind_label(k: &UnitKind) -> &'static str {
    match k {
        UnitKind::Process => "process",
        UnitKind::Docker => "docker",
        UnitKind::Generic => "generic",
    }
}

fn edge_kind_label(k: &UnitEdgeKind) -> &'static str {
    match k {
        UnitEdgeKind::DependsOn => "depends_on",
        UnitEdgeKind::TalksTo => "talks_to",
        UnitEdgeKind::Produces => "produces",
        UnitEdgeKind::Consumes => "consumes",
    }
}

// Quote any string that could otherwise be misread as a YAML scalar (numbers,
// `true`/`false`, leading sigils, embedded `:` / `#`).
fn yaml_string(s: &str) -> String {
    if s.is_empty()
        || s.contains(['"', '\'', ':', '#', '\n', '\\'])
        || s.starts_with([' ', '-', '?', '!', '&', '*'])
        || s.parse::<f64>().is_ok()
        || matches!(
            s.to_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no"
        )
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orkesy_core::config::parse_workspace;

    #[test]
    fn generated_yaml_round_trips_through_canonical_parser() {
        let legacy = r#"
name: roundtrip-app
services:
  api:
    command: ["node", "server.js"]
    port: 3000
    kind: http
    autostart: true
    depends_on: [db]
  db:
    command: ["postgres"]
    port: 5432
"#;
        let cfg = OrkesyConfig::parse(legacy).unwrap();
        let yaml = generate_units_yaml(cfg.project_name(), &cfg.to_units(), &cfg.to_edges());

        let ws = parse_workspace(&yaml)
            .unwrap_or_else(|e| panic!("generated YAML did not parse: {e}\n---\n{yaml}"));

        assert!(!ws.legacy);
        assert_eq!(ws.project_name.as_deref(), Some("roundtrip-app"));
        assert_eq!(ws.units.len(), 2);
        assert_eq!(ws.edges.len(), 1);
    }

    #[test]
    fn yaml_string_quotes_special_values() {
        assert_eq!(yaml_string("hello"), "hello");
        assert_eq!(yaml_string("true"), "\"true\"");
        assert_eq!(yaml_string("123"), "\"123\"");
        assert_eq!(yaml_string("a: b"), "\"a: b\"");
        assert_eq!(yaml_string(""), "\"\"");
    }
}
