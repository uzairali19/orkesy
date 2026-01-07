# Orkesy CLI

A modern, runtime-agnostic terminal UI for managing services, logs, and commands — not just Docker.

Orkesy is an interactive CLI dashboard built in Rust that lets you observe, control, and interact with running services in real time — using a fast, keyboard-first interface inspired by tools like pnpm, htop, and modern AI CLIs.

⸻

# 🚀 What is Orkesy?

Orkesy is not a Docker CLI.
It’s not tied to Node, Ruby, or any single runtime.

Orkesy treats everything as a service:
	•	A Node.js server
	•	A Ruby worker
	•	A Python script
	•	A background job
	•	A shell command
	•	A container (optional)

If it can:
	•	start
	•	stop
	•	emit logs
	•	run commands

# 👉 Orkesy can manage it.

⸻

# 🧠 Why Orkesy?

Most CLIs:
	•	are runtime-specific
	•	hide context
	•	force you to remember flags
	•	don’t scale well as projects grow

Orkesy gives you:
	•	live visibility
	•	interactive control
	•	one consistent UI
	•	zero mouse usage

All from your terminal.

⸻

✨ Features
	•	⚡ Real-time log streaming
	•	⏸️ Pause & inspect logs without stopping ingestion
	•	🧭 Keyboard-first service navigation
	•	/ Command palette with autocomplete
	•	📊 ASCII dependency graph view
	•	🧠 Reducer-based state model
	•	🔌 Pluggable engine architecture
	•	🧪 Fake engine for fast development
	•	🛠 Designed for future runtimes (Node, Ruby, shell, Docker, remote)

⸻

# 🖥️ Interface Overview

Services Pane
	•	Lists all services
	•	Shows live status (starting, running, stopped)
	•	Arrow-key navigation

Right Pane
	•	Live logs (scrollable, pausable)
	•	Graph view for dependencies
	•	Designed to become interactive (selection, actions, drill-down)

Footer
	•	Minimal, always-visible key hints
	•	No overflow, no clutter

Command Palette
	•	Open with /
	•	Autocomplete commands
	•	History navigation
	•	Run commands on one service or all services

⸻

# ⌨️ Keyboard Controls

Key	Action
↑ / ↓	Select service
Space	Pause / resume logs
r	Restart
s	Stop
t	Start
Enter	Toggle
x	Kill
g	Toggle graph
/	Command palette
q	Quit


⸻

# 🧩 Architecture

Orkesy cleanly separates UI, state, and execution.

UI (TUI)
  ↓
Reducer / State
  ↓
Engine (pluggable)

This means:
	•	The UI doesn’t care how a service runs
	•	Engines can be swapped without touching the UI
	•	Future integrations are first-class citizens

⸻

🔮 What This Can Become
	•	Universal dev service manager
	•	Local process supervisor
	•	Runtime-agnostic dashboard
	•	AI-augmented ops CLI
	•	Foundation for platform tooling

Orkesy is intentionally small, composable, and extensible.

⸻

🛠 Built With
	•	Rust
	•	Tokio
	•	ratatui
	•	crossterm

⸻

# 🚀 Getting Started

git clone https://github.com/your-username/orkesy-cli.git
cd orkesy-cli
cargo run


⸻

# 👤 Author

Uzair Ali
	•	GitHub: @uzairali19￼
	•	Twitter: @Uzairali751￼
	•	LinkedIn: Uzair Ali￼

⸻

# 🤝 Contributing

Ideas, issues, and contributions are welcome.
This project is intentionally open-ended — experimentation encouraged.

⸻

# ⭐ Show Your Support

If this project resonates with you, give it a ⭐️
It helps more than you think.

⸻

# 📝 License

MIT License

