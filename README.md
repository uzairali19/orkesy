<h1 align="center">Orkesy</h1>

<p align="center">
  A terminal-based control plane for the services your project runs locally — APIs, workers, databases, build watchers — with one keyboard-driven view of status, logs, health, and resource use.
</p>

<p align="center">
  <a href="https://github.com/uzairali19/orkesy/actions/workflows/ci.yml"><img src="https://github.com/uzairali19/orkesy/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/uzairali19/orkesy/releases"><img src="https://img.shields.io/github/v/release/uzairali19/orkesy?color=blue" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"></a>
</p>

<p align="center">
  <img src="output.gif" width="700" alt="Orkesy demo" />
</p>

---

## What this does

- Runs and supervises multiple services declared in a single `orkesy.yml`
- Live log streaming with search, level filtering, and pause/follow
- Optional persistent log history readable from the CLI
- Per-unit CPU / memory and HTTP / TCP / exec health probes
- Project detection for Node, Python, Rust, Go, Docker Compose
- Config migration from the legacy `services:` schema to the canonical `units:` schema

---

## Why it exists

Local development environments accumulate complexity: five terminal tabs running `npm run dev`, `cargo watch`, `docker compose`, a worker, a tunnel; a half-broken `start.sh` nobody trusts; logs that scroll past you in the wrong window; no shared view of what's healthy or what crashed at 2am. Tools like Procfile, overmind, and docker compose each handle a slice of this. Orkesy aims at the whole shape: declare what runs, see all of it, control it, debug it — without leaving the terminal.

---

## Quick example

```bash
orkesy                           # launch the TUI
orkesy --engine fake             # demo mode, no config required
orkesy init                      # detect the project, write orkesy.yml
orkesy logs api --since 1h       # read persisted history (opt-in)
orkesy migrate                   # rewrite a legacy `services:` config
```

`orkesy.yml` looks like this:

```yaml
version: 1
project: { name: my-app }
units:
  api:
    kind: process
    start: "npm run dev"
    port: 3000
    health: { type: http, url: "http://localhost:3000/health" }
    autostart: true
  db:
    kind: docker
    start: "docker compose up -d postgres"
    stop:  "docker compose stop postgres"
    port: 5432
edges:
  - { from: api, to: db, kind: depends_on }
```

More examples: [`examples/`](examples/).

---

## How it works (high level)

- A YAML config becomes a runtime graph (`Workspace` → `RuntimeGraph`)
- An engine runs each unit through an adapter (process, Docker, fake)
- Lifecycle, health, logs, and metrics flow as events into a pure reducer
- The TUI renders from the resulting state on every tick
- Logs live in bounded in-memory buffers; persisted to disk if `log_history.enabled` is set

Details, diagrams, and the contentious choices: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## What this is not

- **Not a framework.** Your services don't import or link against Orkesy. It launches them as ordinary processes or containers.
- **Not a platform.** No multi-tenancy, no auth, no rolling updates, no orchestrator semantics.
- **Not a one-command deploy tool.** It runs things on your machine, not on a cluster.

This is a reference system you can adapt, not a drop-in solution. If you need cluster orchestration or production-grade log aggregation, use the tools that do those — Orkesy stays out of their way.

---

## Install

### Pre-built binaries

```bash
# macOS — Apple Silicon
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-aarch64-apple-darwin.tar.gz
tar -xzf orkesy-aarch64-apple-darwin.tar.gz && chmod +x orkesy && sudo mv orkesy /usr/local/bin/

# macOS — Intel
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-x86_64-apple-darwin.tar.gz

# Linux — x64
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-x86_64-unknown-linux-gnu.tar.gz

# Linux — arm64
curl -LO https://github.com/uzairali19/orkesy/releases/latest/download/orkesy-aarch64-unknown-linux-gnu.tar.gz

# Windows — download .zip from the releases page and extract
```

All releases: <https://github.com/uzairali19/orkesy/releases>

### From source (cargo)

```bash
cargo install --git https://github.com/uzairali19/orkesy --locked orkesy-cli
```

Or clone and build:

```bash
git clone https://github.com/uzairali19/orkesy.git
cd orkesy && make release         # or: cargo build --release
./target/release/orkesy --help
```

Optional features: `docker` (Docker engine via `bollard`), `health-http` (HTTP probes via `reqwest`). Pass with `--features "docker health-http"`.

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Support

If Orkesy saves you time, consider supporting its development:

<a href="https://buymeacoffee.com/uzairralii" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/v2/default-yellow.png" alt="Buy Me A Coffee" height="40"></a>

---

## License

MIT — see [LICENSE](LICENSE).

---

<p align="center">
  Made by <a href="https://github.com/uzairali19">Uzair Ali</a>
</p>
