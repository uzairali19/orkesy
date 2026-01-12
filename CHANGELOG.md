# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
