# Orkesy Roadmap

This document outlines the planned development roadmap for Orkesy. Features are organized by version milestones with target timeframes. Priorities may shift based on community feedback and contributions.

> **Note:** This roadmap is a living document. Target dates are estimates and subject to change.

---

## Current Release: v0.1.x (Stable)

The foundation release with core functionality for local service management.

### Completed Features
- Cross-platform support (Linux, macOS, Windows)
- Process and Docker container adapters
- Real-time log streaming with timestamps (HH:MM:SS)
- Log level filtering (errors, warnings, all)
- Full-text search with auto-scroll to matches
- Live CPU/memory metrics per service
- Automatic restart policy with exponential backoff
- Health checks (HTTP, TCP, exec-based probes)
- Command palette with fuzzy search
- Project detection: Node.js, Rust, Docker Compose, Make, Just
- Dependency graph visualization
- Demo mode with realistic simulated metrics

---

## v0.2.0 — Enhanced Detection & Init (Q1 2026)

**Theme:** Zero-config setup for more ecosystems

### Project Detection Expansion
| Feature | Description | Status |
|---------|-------------|--------|
| Python detection | Detect `pyproject.toml`, `requirements.txt`, `poetry.lock`, `uv.lock` | 🔄 In Progress |
| Go detection | Detect `go.mod`, parse `go run` targets | 🔄 In Progress |
| Ruby detection | Detect `Gemfile`, Rails applications, `bin/rails` commands | 📋 Planned |
| PHP detection | Detect `composer.json`, Laravel/Symfony frameworks | 📋 Planned |
| Java/Kotlin detection | Detect `pom.xml`, `build.gradle`, Spring Boot apps | 📋 Planned |

### Interactive Init Mode
| Feature | Description | Status |
|---------|-------------|--------|
| TUI service selector | Interactive checkbox UI for selecting which services to include | 🔄 In Progress |
| Config preview | Show generated `orkesy.yml` before writing to disk | 📋 Planned |
| Merge mode | Intelligently merge with existing `orkesy.yml` files | 📋 Planned |
| Template library | Pre-built templates for common stacks (MERN, Django, Rails, etc.) | 📋 Planned |

### Smart Inference
| Feature | Description | Status |
|---------|-------------|--------|
| Port detection | Parse source code and configs for port numbers | 📋 Planned |
| Dependency inference | Detect service dependencies from imports/requires | 📋 Planned |
| Health check suggestions | Auto-suggest health endpoints based on framework | 📋 Planned |
| Environment detection | Detect and document required environment variables | 📋 Planned |

---

## v0.3.0 — Logs & Search Enhancements (Q2 2026)

**Theme:** Professional-grade log management

### Log Management
| Feature | Description | Status |
|---------|-------------|--------|
| Persistent log history | Save logs to disk with configurable retention | 📋 Planned |
| Log export | Export logs to file (JSON, plain text, CSV) | 📋 Planned |
| Log rotation | Automatic rotation based on size/time | 📋 Planned |
| Multi-service log view | Interleaved logs from multiple services with color coding | 📋 Planned |

### Search & Filtering
| Feature | Description | Status |
|---------|-------------|--------|
| Regex search | Full regular expression support in log search | 📋 Planned |
| Time-based filtering | Filter logs by time range (last 5m, 1h, custom) | 📋 Planned |
| Saved filters | Save and recall frequently used filter combinations | 📋 Planned |
| Search history | Navigate through previous search queries | 📋 Planned |

### Log Analysis
| Feature | Description | Status |
|---------|-------------|--------|
| Log rate metrics | Visualize logs/second per service | 📋 Planned |
| Error rate tracking | Track and alert on error rate spikes | 📋 Planned |
| Pattern detection | Identify recurring error patterns | 📋 Planned |
| Stack trace grouping | Group similar stack traces together | 📋 Planned |

---

## v0.4.0 — Remote Services & Kubernetes (Q3 2026)

**Theme:** Beyond local development

### SSH Remote Services
| Feature | Description | Status |
|---------|-------------|--------|
| SSH adapter | Connect to services running on remote hosts via SSH | 📋 Planned |
| SSH key management | Support for SSH keys, agents, and config files | 📋 Planned |
| Remote log streaming | Stream logs from remote services in real-time | 📋 Planned |
| Remote metrics | Collect CPU/memory metrics from remote hosts | 📋 Planned |

### Kubernetes Integration
| Feature | Description | Status |
|---------|-------------|--------|
| K8s adapter | Manage pods, deployments, and services | 📋 Planned |
| Namespace support | Work across multiple Kubernetes namespaces | 📋 Planned |
| Pod log streaming | Stream logs from Kubernetes pods | 📋 Planned |
| K8s metrics | Display pod resource usage from metrics-server | 📋 Planned |
| Port forwarding | Automatic port-forward for local access | 📋 Planned |
| Context switching | Easy switching between K8s contexts | 📋 Planned |

### Container Orchestration
| Feature | Description | Status |
|---------|-------------|--------|
| Docker Swarm | Support for Docker Swarm services | 📋 Planned |
| Podman support | Alternative container runtime support | 📋 Planned |
| Container shell | Interactive shell access to containers | 📋 Planned |

---

## v0.5.0 — Plugin System & Extensibility (Q4 2026)

**Theme:** Make Orkesy your own

### Plugin Architecture
| Feature | Description | Status |
|---------|-------------|--------|
| Plugin API | Stable API for third-party plugins | 📋 Planned |
| Plugin manager | Install, update, remove plugins from CLI | 📋 Planned |
| Plugin registry | Central registry for discovering plugins | 📋 Planned |
| Lua scripting | Lightweight scripting for custom behaviors | 📋 Planned |

### Customization
| Feature | Description | Status |
|---------|-------------|--------|
| Custom keybindings | User-configurable keyboard shortcuts | 📋 Planned |
| Theme system | Customizable color schemes and styles | 📋 Planned |
| Layout presets | Save and restore custom panel layouts | 📋 Planned |
| Custom commands | Define project-specific commands in config | 📋 Planned |

### Integration Plugins (Examples)
| Plugin | Description | Status |
|--------|-------------|--------|
| Slack notifications | Send alerts to Slack channels | 📋 Planned |
| Discord notifications | Send alerts to Discord webhooks | 📋 Planned |
| PagerDuty integration | Escalate critical alerts to PagerDuty | 📋 Planned |
| Datadog export | Export metrics to Datadog | 📋 Planned |
| Prometheus export | Expose metrics in Prometheus format | 📋 Planned |

---

## v1.0.0 — Production Ready (Q1 2027)

**Theme:** Enterprise-grade stability

### Stability & Performance
| Feature | Description | Status |
|---------|-------------|--------|
| Performance audit | Optimize for large service counts (50+) | 📋 Planned |
| Memory optimization | Reduce memory footprint for long-running sessions | 📋 Planned |
| Stress testing | Comprehensive test suite for edge cases | 📋 Planned |
| Crash recovery | Graceful handling of service crashes | 📋 Planned |

### Enterprise Features
| Feature | Description | Status |
|---------|-------------|--------|
| Team configs | Shareable team configuration presets | 📋 Planned |
| Audit logging | Track who started/stopped services | 📋 Planned |
| Role-based access | Control who can perform actions (read-only mode) | 📋 Planned |
| Config validation | Strict validation with helpful error messages | 📋 Planned |

### Documentation & Polish
| Feature | Description | Status |
|---------|-------------|--------|
| Comprehensive docs | Full documentation site | 📋 Planned |
| Video tutorials | Getting started and advanced usage videos | 📋 Planned |
| Migration guides | Guides for migrating from similar tools | 📋 Planned |
| API documentation | Complete API docs for plugin developers | 📋 Planned |

---

## Future Considerations (Beyond v1.0)

These features are being considered for future releases but are not yet scheduled:

### Advanced Features
- **Web UI** — Browser-based dashboard as alternative to TUI
- **Mobile companion** — iOS/Android app for monitoring on the go
- **AI-powered debugging** — Intelligent error analysis and suggestions
- **Distributed tracing** — Integration with OpenTelemetry/Jaeger
- **Service mesh support** — Istio, Linkerd integration

### Platform Expansion
- **VS Code extension** — Integrated Orkesy panel in VS Code
- **JetBrains plugin** — Support for IntelliJ, WebStorm, etc.
- **Neovim integration** — Native Neovim plugin
- **CI/CD integration** — GitHub Actions, GitLab CI support

### Community
- **Plugin marketplace** — Community-contributed plugins
- **Config sharing** — Share and discover orkesy.yml configurations
- **Usage analytics** — Opt-in anonymous usage statistics

---

## Version History

| Version | Release Date | Highlights |
|---------|--------------|------------|
| v0.1.0 | Jan 2026 | Initial release with core features |

---

## Contributing to the Roadmap

We welcome community input on the roadmap! Here's how you can contribute:

1. **Feature requests** — Open an issue with the `enhancement` label
2. **Vote on features** — React with 👍 on issues you want prioritized
3. **Discuss** — Join discussions in the GitHub Discussions tab
4. **Contribute** — Pick up a planned feature and submit a PR

### Priority Factors

Features are prioritized based on:
- Community demand (issue reactions and comments)
- Strategic alignment with project goals
- Implementation complexity vs. user value
- Contributor availability

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Completed |
| 🔄 | In Progress |
| 📋 | Planned |
| 🔮 | Under Consideration |

---

<p align="center">
  <i>Last updated: January 2026</i>
</p>
