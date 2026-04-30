# Contributing to Orkesy

Thanks for considering a contribution. This document covers the practical bits: how to build, where things live, and what kinds of changes are easiest to land.

For *what the project is and why it's shaped the way it is*, see the [README](README.md) and [ARCHITECTURE.md](docs/ARCHITECTURE.md). Read those first.

---

## Building and running

```bash
git clone https://github.com/uzairali19/orkesy.git
cd orkesy

# Standard dev cycle
cargo build                          # debug build
cargo run -- --engine fake           # demo mode, no config required
cargo test --workspace               # all tests
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check

# Optional features
cargo build --features "docker health-http"
```

Minimum supported toolchain: stable Rust matching the version pinned in CI.

---

## Repository layout

```
orkesy-core/   # pure library: model, reducer, config, state, metrics
orkesy-cli/    # binary `orkesy`: TUI, engines, adapters, detectors, health
docs/          # ARCHITECTURE, DECISIONS, ROADMAP, LOG_HISTORY_DESIGN
examples/      # sample orkesy.yml configs
.github/       # CI, issue / PR templates
```

If you are not sure which crate your change belongs in, the rule of thumb:

> Does the code touch the OS, the terminal, or the network? It belongs in `orkesy-cli`. Otherwise it belongs in `orkesy-core`.

`orkesy-core` is deliberately I/O-free.

---

## Development workflow

1. **Open an issue first** for non-trivial changes. A short conversation about approach is faster than a rejected PR.
2. **Branch from `main`** — `git checkout -b feature/short-name` or `fix/short-name`.
3. **Keep PRs small.** One concern per PR. A renamed function and a new feature should not share a PR.
4. **Add tests.** New parsing, new reducer arms, and new validation paths should have unit tests in the same file.
5. **Update docs.** If you add a config field, update the README example and `examples/`. If you change the data flow, update `docs/ARCHITECTURE.md`.
6. **Run the full check locally:**
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --all-targets
   ```
7. **Open a PR** against `main`. Fill in the template. Link the issue if there is one.

CI runs the same three checks plus a build for all five release targets, so a PR that passes locally usually passes upstream.

---

## What kind of changes are we looking for

### Easy to land

- New project detector under `orkesy-cli/src/detectors/` (Ruby, PHP, Java/Kotlin, etc.).
- New health-check type (e.g. gRPC) in `orkesy-cli/src/health.rs`.
- Better validation errors in `orkesy-core/src/config.rs`.
- Documentation, examples, and `examples/*.yaml`.
- Bug fixes with a test that demonstrates the bug.

### Discuss first

- New events on `RuntimeEvent` — they are part of the project's contract.
- New top-level config fields. We try to keep the schema small.
- New dependencies. Each one is a maintenance commitment; defaults to "no".
- Anything that would change the "local-first, runtime-agnostic" framing — see "Key Decisions & Tradeoffs" in [ARCHITECTURE.md](docs/ARCHITECTURE.md).

### Probably not

- Mouse support in the TUI. (Keyboard-only is intentional.)
- A web UI as the primary interface. (TUI-first is intentional.)
- Built-in alerting / paging integrations. (Out of scope; plugin territory.)

---

## Good first issues

A few tractable starting points if you're new to the codebase:

1. **Add a new detector.** Pick an ecosystem (Ruby, PHP, .NET) and write `orkesy-cli/src/detectors/<lang>.rs` matching the existing patterns. Wire it into `commands/init.rs::scan_project`.
2. **Add an `examples/` config** for a stack you know well (Django + Celery + Redis, Phoenix + Postgres, etc.).
3. **Improve a `ConfigError` variant.** Pick one where the message could be clearer (e.g. `CyclicDependency` could underline which edge to remove). Add a test that asserts the new message.
4. **Add a unit test for the reducer.** Pick a `RuntimeEvent` variant and write a test that constructs an `EventEnvelope` and asserts the resulting `RuntimeState`.
5. **Surface a missing keybinding in `?` help.** The help screen is in the views layer; check that every binding in `input.rs` is documented.

Issues labelled [`good first issue`](https://github.com/uzairali19/orkesy/labels/good%20first%20issue) on GitHub are explicitly scoped for first contributions.

---

## Code style

- **Idiomatic Rust.** Prefer `match` over `if let` chains. Prefer `?` over `unwrap` outside tests. Prefer typed errors (`thiserror`-style enums) over `String`s when the error has cases.
- **No emojis in code or commits.** They show up oddly in some terminals.
- **Comments explain *why*, not *what*.** If the code is self-evident, no comment. If it isn't, name the thing that's not obvious (a constraint, a workaround, a perf decision).
- **One public type per file** is a soft rule; don't fight it when grouping a small enum with its impls makes more sense.
- **Tests live next to code** in `#[cfg(test)] mod tests { ... }` blocks. Integration tests live under `tests/`.

We rely on `cargo fmt` and `cargo clippy -- -D warnings` to enforce the basics. If clippy flags something and you disagree, add an `#[allow(...)]` with a one-line comment explaining why, rather than ignoring the lint repo-wide.

---

## Commit messages

Imperative mood, lowercased subject under 72 chars, body wrapped at 72:

```
add Python detection for poetry projects

Detect `pyproject.toml` with `[tool.poetry]`, generate a unit using
`poetry run` for the start command. Falls back to pip if poetry isn't
present.
```

We don't enforce conventional commits, but `add:`, `fix:`, `docs:` prefixes are welcome.

---

## Releasing

Maintainer-only:

1. Bump version in workspace `Cargo.toml`.
2. Update `CHANGELOG.md`.
3. `git tag vX.Y.Z && git push origin vX.Y.Z`.
4. The release workflow builds and publishes binaries for all five targets.

---

## Code of conduct

Be kind, be specific, prefer asking to assuming. We're all here to make a useful tool.

---

## Questions

- Open a [Discussion](https://github.com/uzairali19/orkesy/discussions) for general questions.
- Open an Issue for bugs or feature requests with reproducible scope.
- Mention a maintainer in a PR if it's been sitting more than a week.
