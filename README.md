# Orkesy

A terminal-based control plane for the services your project runs locally — APIs, workers, databases, build watchers — with one keyboard-driven view of status, logs, health, and resource use.

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

Pre-built binaries on the [releases page](https://github.com/uzairali19/orkesy/releases) for Linux (x64, arm64), macOS (Intel, Apple Silicon), and Windows (x64). Or build from source:

```bash
git clone https://github.com/uzairali19/orkesy.git
cd orkesy && cargo build --release
./target/release/orkesy --help
```

Optional features: `docker` (Docker engine via `bollard`), `health-http` (HTTP probes via `reqwest`).

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). MIT license, [LICENSE](LICENSE).
