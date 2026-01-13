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

<!-- Screenshot placeholder - add a demo GIF here -->
<!-- <p align="center">
  <img src="assets/demo.gif" width="700" alt="Orkesy demo" />
</p> -->

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

| Set | Description |
|---|---|
| ⚡ **Real-time logs** | Stream, pause, scroll, search, filter |
| 📊 **Live metrics** | CPU, memory, network, log rate charts |
| ⌨️ **Command palette** | Fuzzy search with `/` (VS Code style) |
| 🔄 **Lifecycle control** | Start, stop, restart, kill services |
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

- [ ] Remote services (SSH, Kubernetes)
- [ ] Persistent metrics history
- [ ] Custom keybindings
- [ ] Theme customization
- [ ] Plugin system
- [ ] Notifications & alerts

---

## Contributing

Contributions welcome! Please open an issue first to discuss changes.

```bash
cargo test              # Run tests
cargo clippy            # Lint
cargo fmt               # Format
```

---

## License

MIT — see [LICENSE](LICENSE)

---

<p align="center">
  Made by <a href="https://github.com/uzairali19">Uzair Ali</a>
</p>
