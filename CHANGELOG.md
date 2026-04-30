# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-04-30

### Added
- **Canonical `units:` config schema.** Now the documented public surface; `services:` continues to load as a legacy fallback with a one-line deprecation warning. Both shapes converge on the same internal `Workspace` before the runtime sees them. (`orkesy-core/src/config.rs`)
- **`orkesy migrate` subcommand.** Rewrites a legacy `services:` file into the canonical `units:` form, with a timestamped `.bak` of the original. Idempotent.
- **Persistent log history (writer + reader).** Opt-in via `log_history.enabled: true` in `orkesy.yml`. Writes per-project per-hour JSONL files under `$XDG_STATE_HOME/orkesy/projects/<hash>/logs/`. Read back via `orkesy logs <unit> --since <duration> --level <error|warn|all> --format <text|json>`. See `docs/ARCHITECTURE.md` for the design.
- **Modular CLI layout.** `main.rs` shrunk by ~1,000 LOC; new modules: `app`, `input`, `render`, `views/{command_palette,help,search_bar,status_bar,footer}`, `log_history`, `commands/migrate`.
- **Documentation:** `docs/ARCHITECTURE.md` (with a "Key Decisions & Tradeoffs" section), `CONTRIBUTING.md`.
- **Makefile** with the common dev targets (`build`, `test`, `lint`, `ci`, `release`, `install`, `clean`).
- **Tests:** ~30 new unit and integration tests covering canonical schema, legacy round-trip, palette filtering, command parsing, log-history writer + reader.

### Fixed
- **Latent config bug:** `examples/*.yaml` and the output of `orkesy init` use the `units:` schema, but the loader only recognised `services:`. Affected configs silently fell through to demo mode. The unified parser now accepts both.

### Changed
- **Better config error messages.** `ConfigError` variants now name the offending unit and suggest a remediation. `NotFound` lists the searched paths and points at `orkesy init`.

## [0.1.1] - 2026-01-15

### Added
- Log timestamps (HH:MM:SS format) on every log line
- Log level filtering with keyboard shortcuts (`e` errors, `w` warn+, `a` all)
- Search auto-scroll to first match when typing
- Automatic restart policy with exponential backoff (max 3 restarts per 60s)
- Enhanced demo mode with realistic CPU/memory metrics
- Demo mode error/warning log messages for showcasing filters
- Demo mode incrementing job counter for worker service
- Detailed roadmap with version milestones (docs/ROADMAP.md)
- Contributing guide and Buy Me a Coffee section in README

### Fixed
- Windows build compatibility (Unix-specific code now properly gated)
- Search now scans all logs, not just visible viewport
- Metrics display now shows actual values (was showing zeros)
- Clean pipelines for github workflows

### Changed
- CI workflow now tests all 5 release targets before releases
- README roadmap section now links to detailed roadmap document
- Cleaned up code comments for production readiness

## [0.1.0] - 2025-01-12

### Added
- Initial release of Orkesy TUI
- Real-time log streaming with pause, scroll, and search
- Live metrics charts (CPU, Memory, Network, Log Rate) with 60-second window
- VS Code-style command palette with fuzzy search
- Service lifecycle management (start, stop, restart, kill)
- Health checks: HTTP, TCP, and Exec-based
- Dependency graph visualization
- Project detection for Node.js, Rust, Docker Compose, Make, Just
- Multiple backend engines: local process, Docker (optional), fake (testing)
- `orkesy init` command for project initialization
- `orkesy doctor` command for setup verification
- Keyboard-first interface with Tab focus cycling
- "all" aggregate view for services
- 46 unit tests across core modules
