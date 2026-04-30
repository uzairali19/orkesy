use orkesy_core::config::{OrkesyConfig, parse_workspace};

const LEGACY_YAML: &str = r#"
name: legacy-app
services:
  api:
    command: ["node", "server.js"]
    port: 8000
    kind: http
    autostart: true
    depends_on: [db]
  db:
    command: ["postgres"]
    port: 5432
    kind: database
"#;

#[test]
fn legacy_path_and_unified_path_agree() {
    let legacy = OrkesyConfig::parse(LEGACY_YAML).unwrap();
    let legacy_units = legacy.to_units();
    let legacy_edges = legacy.to_edges();
    let legacy_name = legacy.project_name().map(|s| s.to_string());

    let ws = parse_workspace(LEGACY_YAML).unwrap();

    assert!(ws.legacy, "should be detected as legacy schema");
    assert_eq!(ws.project_name, legacy_name);

    let mut a: Vec<&str> = ws.units.iter().map(|u| u.id.as_str()).collect();
    let mut b: Vec<&str> = legacy_units.iter().map(|u| u.id.as_str()).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b);

    let mut ea: Vec<(String, String)> = ws
        .edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    let mut eb: Vec<(String, String)> = legacy_edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    ea.sort();
    eb.sort();
    assert_eq!(ea, eb);
}

#[test]
fn legacy_validation_still_runs() {
    let yaml = r#"
services:
  a: { command: ["echo"], depends_on: [b] }
  b: { command: ["echo"], depends_on: [a] }
"#;
    let err = parse_workspace(yaml).unwrap_err();
    assert!(matches!(
        err,
        orkesy_core::config::ConfigError::CyclicDependency { .. }
    ));
}
