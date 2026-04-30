// Every shipped example must parse through the canonical loader. If a user
// copies one of these and it does not load, the bug is ours.

use orkesy_core::config::parse_workspace;
use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap().join("examples")
}

fn parse_example(name: &str) {
    let path = examples_dir().join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let ws = parse_workspace(&content)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));
    assert!(
        !ws.units.is_empty(),
        "{} parsed but produced no units",
        path.display()
    );
    assert!(
        !ws.legacy,
        "{} should use the canonical `units:` schema, not legacy",
        path.display()
    );
}

#[test]
fn simple_yaml_parses() {
    parse_example("simple.yaml");
}

#[test]
fn webdev_yaml_parses() {
    parse_example("webdev.yaml");
}

#[test]
fn docker_yaml_parses() {
    parse_example("docker.yaml");
}

#[test]
fn monorepo_yaml_parses() {
    parse_example("monorepo.yaml");
}
