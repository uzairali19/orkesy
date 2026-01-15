<h1 align="center">Orkesy</h1>

<p align="center">
  <strong>A modern, runtime-agnostic terminal UI for managing services, logs, and metrics.</strong>
</p>

<p align="center">
  <a href="https://github.com/uzairali19/orkesy/actions/workflows/ci.yml"><img src="https://github.com/uzairali19/orkesy/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/uzairali19/orkesy/releases"><img src="https://img.shields.io/github/v/release/uzairali19/orkesy?color=blue" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"></a>
</p>

<p align="center">
  <a href="https://github.com/uzairali19/orkesy/releases">Releases</a> •
  <a href="#installation">Install</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#features">Features</a> •
  <a href="#configuration">Config</a>
</p>

---

<p align="center">
  <img src="output.gif" width="700" alt="Orkesy demo" />
</p>

## What is Orkesy?

**Orkesy** is an interactive CLI dashboard that lets you observe, control, and interact with running services in real time. Built in Rust with a fast, keyboard-first interface inspired by **htop**, **lazydocker**, and **VS Code**.

It's **runtime-agnostic** - if it can start, stop, and emit logs, Orkesy can manage it:


- Node.js servers
- Rust APIs
- Python workers
- Docker containers
- Background jobs
- Shell commands


---

## Features

| Feature | Description |
|---|---|
| ⚡ **Real-time logs** | Stream, pause, search, filter by level (error/warn/all) |
| 🕐 **Timestamps** | HH:MM:SS timestamps on every log line |
| 📊 **Live metrics** | CPU, memory, uptime per service |
| ⌨️ **Command palette** | Fuzzy search with `/` (VS Code style) |
| 🔄 **Lifecycle control** | Start, stop, restart, kill with auto-restart policy |
| ❤️ **Health checks** | HTTP, TCP, and exec-based probes |
| 🧩 **Dependency graph** | Visualize service relationships |
| 🔍 **Auto-detection** | Node, Rust, Docker Compose, Make, Just |

**TUI:** Adaptive layout • Panel focus model • VS Code dark theme • Keyboard-first

---

## Installation

### Download binary (recommended)

```bash
# macOS (Apple Silicon)
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-aarch64-apple-darwin.tar.gz
tar -xzf orkesy-aarch64-apple-darwin.tar.gz
chmod +x orkesy && sudo mv orkesy /usr/local/bin/

# macOS (Intel)
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-x86_64-apple-darwin.tar.gz

# Linux (x64)
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-x86_64-unknown-linux-gnu.tar.gz

# Windows — download .zip from Releases and extract
```

**[All releases →](https://github.com/uzairali19/orkesy/releases)**

### Build from source

```bash
git clone https://github.com/uzairali19/orkesy.git && cd orkesy
cargo build --release
./target/release/orkesy --version
```

---

## Quick Start

```bash
orkesy init          # Detect project, generate orkesy.yml
orkesy               # Launch TUI
orkesy doctor        # Check setup
orkesy --engine fake # Demo mode (no config needed)
```

---

## Configuration

Create `orkesy.yml` in your project root:

```yaml
project: my-app

units:
  api:
    kind: process
    start: npm run dev
    port: 3000
    health:
      http:
        path: /health
        interval_ms: 5000

  worker:
    kind: process
    start: node worker.js
    depends_on: [api]

  db:
    kind: docker
    start: docker compose up -d postgres
    port: 5432
```

> **Tip:** `orkesy init` will auto-generate this for most projects.

---

## Keyboard Controls

### Global

| Key | Action |
|-----|--------|
| `Tab` | Cycle focus |
| `/` | Command palette |
| `?` | Help |
| `q` | Quit |

### Units Panel

| Key | Action |
|-----|--------|
| `↑↓` | Navigate |
| `r` | Restart |
| `s` | Stop |
| `t` | Start |
| `x` | Kill |
| `c` | Clear logs |

### Logs

| Key | Action |
|-----|--------|
| `Space` | Pause/resume |
| `f` | Follow mode |
| `s` | Search |
| `n/N` | Next/prev match |
| `e` | Filter: errors only |
| `w` | Filter: warn and above |
| `a` | Filter: all levels |

### Views

| Key | View |
|-----|------|
| `l` | Logs |
| `i` | Inspect |
| `d` | Dependencies |
| `m` | Metrics |

---

## Architecture

```
orkesy/
├── orkesy-core/             # Library crate
│   ├── model.rs             # Service graph, status types
│   ├── state.rs             # Runtime state, log storage
│   ├── reducer.rs           # Event → state mutations
│   ├── config.rs            # YAML config parsing
│   ├── metrics.rs           # Time-series ring buffers
│   ├── command.rs           # Command registry, palette model
│   ├── unit.rs              # Unit definition, metrics
│   ├── adapter.rs           # Adapter traits
│   ├── engine.rs            # Engine traits
│   ├── job.rs               # Job execution model
│   ├── plugin.rs            # Plugin system
│   └── log_filter.rs        # Log level detection
│
└── orkesy-cli/              # Binary crate
    ├── main.rs              # TUI event loop, rendering
    ├── sampler.rs           # Background metrics collection
    ├── health.rs            # Health check execution
    ├── runner.rs            # Command runner
    ├── engines/
    │   ├── local_process.rs # Local process engine
    │   ├── docker.rs        # Docker engine
    │   └── fake.rs          # Fake engine (testing/demo)
    ├── adapters/
    │   ├── process.rs       # Process management
    │   └── docker.rs        # Docker container management
    ├── detectors/
    │   ├── node.rs          # Node.js detection
    │   ├── rust.rs          # Rust detection
    │   └── docker.rs        # Docker Compose detection
    ├── commands/
    │   ├── init.rs          # orkesy init
    │   └── doctor.rs        # orkesy doctor
    └── ui/
        └── theme.rs         # Color palette, styles
```

**Event flow:** `Input → Event → Reducer → State → Render`

---

## Platforms

| Platform | Target | Archive |
|----------|--------|---------|
| Linux x64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x64 | `x86_64-pc-windows-msvc` | `.zip` |

---

## Built With

- [Rust](https://www.rust-lang.org/) — Systems programming
- [Tokio](https://tokio.rs/) — Async runtime
- [ratatui](https://ratatui.rs/) — Terminal UI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) — Cross-platform terminal
- [sysinfo](https://github.com/GuillaumeGomez/sysinfo) — System metrics

---

## Roadmap

See the **[detailed roadmap](docs/ROADMAP.md)** for version milestones and planned features.

### Upcoming Releases

| Version | Target | Theme |
|---------|--------|-------|
| **v0.2.0** | Q1 2026 | Enhanced Detection & Init — Python, Go, Ruby, PHP detection; interactive init mode |
| **v0.3.0** | Q2 2026 | Logs & Search — Persistent history, regex search, log export, time filtering |
| **v0.4.0** | Q3 2026 | Remote Services — SSH adapter, Kubernetes integration, pod management |
| **v0.5.0** | Q4 2026 | Plugin System — Custom keybindings, themes, Lua scripting, integrations |
| **v1.0.0** | Q1 2027 | Production Ready — Enterprise features, performance optimization, full docs |

### Current Focus (v0.2.0)
- [ ] Python project detection (pyproject.toml, poetry, uv)
- [ ] Go project detection (go.mod)
- [ ] Interactive `orkesy init` with TUI-based service selection
- [ ] Ruby/PHP project detection

### Recently Completed
- [x] Cross-platform support (Linux, macOS, Windows)
- [x] Real-time log streaming with timestamps and level filtering
- [x] Search with auto-scroll to matches
- [x] Live CPU/memory metrics per service
- [x] Automatic restart policy with backoff
- [x] Health checks (HTTP, TCP, exec)
- [x] Command palette with fuzzy search
- [x] Project detection: Node.js, Rust, Docker Compose, Make, Just

---

## Contributing

Contributions are welcome! Whether it's bug fixes, new features, or documentation improvements.

### Getting Started

```bash
# Clone the repo
git clone https://github.com/uzairali19/orkesy.git
cd orkesy

# Build and run
cargo build
cargo run -- --engine fake   # Demo mode

# Run checks
cargo test                   # Run tests
cargo clippy                 # Lint
cargo fmt                    # Format
```

### Development Workflow

1. **Fork** the repository
2. **Create a branch** for your feature (`git checkout -b feature/my-feature`)
3. **Make changes** and add tests if applicable
4. **Run checks** — ensure `cargo test`, `cargo clippy`, and `cargo fmt` pass
5. **Commit** with a clear message
6. **Open a PR** against `main`

### Good First Issues

Look for issues labeled [`good first issue`](https://github.com/uzairali19/orkesy/labels/good%20first%20issue) — these are great starting points for new contributors.

### Areas for Contribution

- **New detectors** — Add support for Python, Go, Ruby, or other ecosystems in `orkesy-cli/src/detectors/`
- **Health check types** — Extend health probes in `orkesy-cli/src/health.rs`
- **UI improvements** — Enhance the TUI in `orkesy-cli/src/main.rs` and `orkesy-cli/src/ui/`
- **Documentation** — Improve README, add examples, or write guides
- **Bug fixes** — Check open issues for reported bugs

---

## Support

If you find Orkesy useful, consider supporting its development:

<a href="https://buymeacoffee.com/uzairralii" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" height="40"></a>

---

## License

MIT — see [LICENSE](LICENSE)

---

<p align="center">
  Made by <a href="https://github.com/uzairali19">Uzair Ali</a>
</p>
