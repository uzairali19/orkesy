use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc};

use orkesy_core::config::ServiceConfig;
use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::{RuntimeGraph, ServiceId, ServiceStatus};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::LogStream;

struct ProcessHandle {
    child: Child,
    pgid: i32,
}

pub struct LocalProcessEngine {
    configs: BTreeMap<ServiceId, ServiceConfig>,
    processes: BTreeMap<ServiceId, ProcessHandle>,
    next_id: Arc<AtomicU64>,
}

impl LocalProcessEngine {
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
            processes: BTreeMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Set service configurations from loaded config
    pub fn with_configs(mut self, configs: BTreeMap<ServiceId, ServiceConfig>) -> Self {
        self.configs = configs;
        self
    }

    fn next_event_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn emit(&self, event_tx: &broadcast::Sender<EventEnvelope>, event: RuntimeEvent) {
        let _ = event_tx.send(EventEnvelope {
            id: self.next_event_id(),
            at: SystemTime::now(),
            event,
        });
    }

    /// Spawn a service process
    async fn spawn_service(
        &mut self,
        id: &ServiceId,
        event_tx: &broadcast::Sender<EventEnvelope>,
    ) -> Result<(), String> {
        let config = self
            .configs
            .get(id)
            .ok_or_else(|| format!("no config for service: {}", id))?;

        if config.command.is_empty() {
            return Err("empty command".into());
        }

        // Build the command
        let mut cmd = Command::new(&config.command[0]);
        cmd.args(&config.command[1..]);

        // Set working directory
        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }

        // Set environment variables
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // Create new process group using pre_exec
        // This allows us to kill the entire process tree later
        unsafe {
            cmd.pre_exec(|| {
                // Create new session (and process group)
                libc::setsid();
                Ok(())
            });
        }

        // Capture stdout and stderr
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Prevent child from inheriting stdin
        cmd.stdin(Stdio::null());

        // Spawn the process
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;

        let pgid = child.id().map(|pid| pid as i32).unwrap_or(-1);

        // Spawn stdout reader task
        if let Some(stdout) = child.stdout.take() {
            let tx = event_tx.clone();
            let service_id = id.clone();
            let next_id = self.next_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(EventEnvelope {
                        id: next_id.fetch_add(1, Ordering::SeqCst),
                        at: SystemTime::now(),
                        event: RuntimeEvent::LogLine {
                            id: service_id.clone(),
                            stream: LogStream::Stdout,
                            text: line,
                        },
                    });
                }
            });
        }

        // Spawn stderr reader task
        if let Some(stderr) = child.stderr.take() {
            let tx = event_tx.clone();
            let service_id = id.clone();
            let next_id = self.next_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(EventEnvelope {
                        id: next_id.fetch_add(1, Ordering::SeqCst),
                        at: SystemTime::now(),
                        event: RuntimeEvent::LogLine {
                            id: service_id.clone(),
                            stream: LogStream::Stderr,
                            text: line,
                        },
                    });
                }
            });
        }

        self.processes
            .insert(id.clone(), ProcessHandle { child, pgid });
        Ok(())
    }

    /// Kill a service's process group
    async fn kill_service(&mut self, id: &ServiceId, graceful: bool) -> Result<(), String> {
        if let Some(mut handle) = self.processes.remove(id) {
            if handle.pgid > 0 {
                // Send SIGTERM to process group
                unsafe {
                    libc::killpg(handle.pgid, libc::SIGTERM);
                }

                if graceful {
                    // Give it time to gracefully shutdown
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // Check if still running
                    if handle.child.try_wait().ok().flatten().is_none() {
                        // Force kill
                        unsafe {
                            libc::killpg(handle.pgid, libc::SIGKILL);
                        }
                    }
                } else {
                    // Immediate kill
                    unsafe {
                        libc::killpg(handle.pgid, libc::SIGKILL);
                    }
                }
            } else {
                // Fallback: just kill the child
                let _ = handle.child.kill().await;
            }
            Ok(())
        } else {
            Err("process not running".into())
        }
    }
}

impl Default for LocalProcessEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for LocalProcessEngine {
    fn name(&self) -> &'static str {
        "local-process"
    }

    async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<EngineCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
        graph: RuntimeGraph,
    ) {
        // Emit topology loaded
        self.emit(
            &event_tx,
            RuntimeEvent::TopologyLoaded {
                graph: graph.clone(),
            },
        );

        // Auto-start services marked for autostart
        for (id, node) in &graph.nodes {
            if matches!(node.desired, orkesy_core::model::DesiredState::Running) {
                self.emit(
                    &event_tx,
                    RuntimeEvent::StatusChanged {
                        id: id.clone(),
                        status: ServiceStatus::Starting,
                    },
                );

                match self.spawn_service(id, &event_tx).await {
                    Ok(()) => {
                        self.emit(
                            &event_tx,
                            RuntimeEvent::StatusChanged {
                                id: id.clone(),
                                status: ServiceStatus::Running,
                            },
                        );
                    }
                    Err(e) => {
                        self.emit(
                            &event_tx,
                            RuntimeEvent::StatusChanged {
                                id: id.clone(),
                                status: ServiceStatus::Errored { message: e.clone() },
                            },
                        );
                        self.emit(
                            &event_tx,
                            RuntimeEvent::LogLine {
                                id: id.clone(),
                                stream: LogStream::System,
                                text: format!("[error] failed to start: {}", e),
                            },
                        );
                    }
                }
            }
        }

        // Main loop: process commands + monitor child exits
        let mut check_interval = tokio::time::interval(Duration::from_millis(100));
        check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = check_interval.tick() => {
                    // Check for exited processes
                    let mut exited = vec![];
                    for (id, handle) in &mut self.processes {
                        if let Ok(Some(status)) = handle.child.try_wait() {
                            exited.push((id.clone(), status.code()));
                        }
                    }

                    for (id, code) in exited {
                        self.processes.remove(&id);
                        self.emit(
                            &event_tx,
                            RuntimeEvent::StatusChanged {
                                id: id.clone(),
                                status: ServiceStatus::Exited { code },
                            },
                        );
                        self.emit(
                            &event_tx,
                            RuntimeEvent::LogLine {
                                id,
                                stream: LogStream::System,
                                text: format!("process exited with code: {:?}", code),
                            },
                        );

                        // TODO: Handle restart policy here
                    }
                }

                cmd = command_rx.recv() => {
                    let Some(cmd) = cmd else { break };

                    match cmd {
                        EngineCommand::Shutdown => {
                            // Kill all running processes
                            let ids: Vec<_> = self.processes.keys().cloned().collect();
                            for id in ids {
                                let _ = self.kill_service(&id, true).await;
                            }
                            break;
                        }

                        EngineCommand::Start { id } => {
                            if self.processes.contains_key(&id) {
                                self.emit(
                                    &event_tx,
                                    RuntimeEvent::LogLine {
                                        id,
                                        stream: LogStream::System,
                                        text: "[warn] already running".into(),
                                    },
                                );
                                continue;
                            }

                            self.emit(
                                &event_tx,
                                RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Starting,
                                },
                            );

                            match self.spawn_service(&id, &event_tx).await {
                                Ok(()) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id,
                                            status: ServiceStatus::Running,
                                        },
                                    );
                                }
                                Err(e) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id: id.clone(),
                                            status: ServiceStatus::Errored { message: e.clone() },
                                        },
                                    );
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: format!("[error] {}", e),
                                        },
                                    );
                                }
                            }
                        }

                        EngineCommand::Stop { id } => {
                            self.emit(
                                &event_tx,
                                RuntimeEvent::LogLine {
                                    id: id.clone(),
                                    stream: LogStream::System,
                                    text: "stopping...".into(),
                                },
                            );

                            match self.kill_service(&id, true).await {
                                Ok(()) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id,
                                            status: ServiceStatus::Stopped,
                                        },
                                    );
                                }
                                Err(e) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: format!("[warn] {}", e),
                                        },
                                    );
                                }
                            }
                        }

                        EngineCommand::Restart { id } => {
                            self.emit(
                                &event_tx,
                                RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Restarting,
                                },
                            );

                            // Kill existing process
                            let _ = self.kill_service(&id, true).await;

                            // Small delay before restart
                            tokio::time::sleep(Duration::from_millis(100)).await;

                            // Start again
                            match self.spawn_service(&id, &event_tx).await {
                                Ok(()) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id: id.clone(),
                                            status: ServiceStatus::Running,
                                        },
                                    );
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: "restarted".into(),
                                        },
                                    );
                                }
                                Err(e) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id: id.clone(),
                                            status: ServiceStatus::Errored { message: e.clone() },
                                        },
                                    );
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: format!("[error] restart failed: {}", e),
                                        },
                                    );
                                }
                            }
                        }

                        EngineCommand::Kill { id } => {
                            match self.kill_service(&id, false).await {
                                Ok(()) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id: id.clone(),
                                            status: ServiceStatus::Stopped,
                                        },
                                    );
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: "killed".into(),
                                        },
                                    );
                                }
                                Err(e) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: format!("[warn] {}", e),
                                        },
                                    );
                                }
                            }
                        }

                        EngineCommand::Toggle { id } => {
                            if self.processes.contains_key(&id) {
                                // Running -> Stop
                                self.emit(
                                    &event_tx,
                                    RuntimeEvent::LogLine {
                                        id: id.clone(),
                                        stream: LogStream::System,
                                        text: "stopping...".into(),
                                    },
                                );
                                let _ = self.kill_service(&id, true).await;
                                self.emit(
                                    &event_tx,
                                    RuntimeEvent::StatusChanged {
                                        id,
                                        status: ServiceStatus::Stopped,
                                    },
                                );
                            } else {
                                // Stopped -> Start
                                self.emit(
                                    &event_tx,
                                    RuntimeEvent::StatusChanged {
                                        id: id.clone(),
                                        status: ServiceStatus::Starting,
                                    },
                                );
                                match self.spawn_service(&id, &event_tx).await {
                                    Ok(()) => {
                                        self.emit(
                                            &event_tx,
                                            RuntimeEvent::StatusChanged {
                                                id,
                                                status: ServiceStatus::Running,
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        self.emit(
                                            &event_tx,
                                            RuntimeEvent::StatusChanged {
                                                id: id.clone(),
                                                status: ServiceStatus::Errored { message: e.clone() },
                                            },
                                        );
                                        self.emit(
                                            &event_tx,
                                            RuntimeEvent::LogLine {
                                                id,
                                                stream: LogStream::System,
                                                text: format!("[error] {}", e),
                                            },
                                        );
                                    }
                                }
                            }
                        }

                        EngineCommand::ClearLogs { id } => {
                            self.emit(&event_tx, RuntimeEvent::ClearLogs { id });
                        }

                        EngineCommand::Exec { id, cmd } => {
                            let shown = cmd.join(" ");
                            self.emit(
                                &event_tx,
                                RuntimeEvent::LogLine {
                                    id: id.clone(),
                                    stream: LogStream::System,
                                    text: format!("$ {}", shown),
                                },
                            );

                            if cmd.is_empty() {
                                self.emit(
                                    &event_tx,
                                    RuntimeEvent::LogLine {
                                        id,
                                        stream: LogStream::System,
                                        text: "[error] empty command".into(),
                                    },
                                );
                                continue;
                            }

                            // Execute the command
                            let output = tokio::process::Command::new(&cmd[0])
                                .args(&cmd[1..])
                                .output()
                                .await;

                            match output {
                                Ok(out) => {
                                    // Emit stdout lines
                                    if !out.stdout.is_empty() {
                                        if let Ok(s) = String::from_utf8(out.stdout) {
                                            for line in s.lines() {
                                                self.emit(
                                                    &event_tx,
                                                    RuntimeEvent::LogLine {
                                                        id: id.clone(),
                                                        stream: LogStream::Stdout,
                                                        text: line.to_string(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    // Emit stderr lines
                                    if !out.stderr.is_empty() {
                                        if let Ok(s) = String::from_utf8(out.stderr) {
                                            for line in s.lines() {
                                                self.emit(
                                                    &event_tx,
                                                    RuntimeEvent::LogLine {
                                                        id: id.clone(),
                                                        stream: LogStream::Stderr,
                                                        text: line.to_string(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    // Emit exit status
                                    let status = if out.status.success() {
                                        "ok".to_string()
                                    } else {
                                        format!("exit code: {:?}", out.status.code())
                                    };
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine { id, stream: LogStream::System, text: status },
                                    );
                                }
                                Err(e) => {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id,
                                            stream: LogStream::System,
                                            text: format!("[error] {}", e),
                                        },
                                    );
                                }
                            }
                        }

                        EngineCommand::EmitLog { id, text } => {
                            self.emit(&event_tx, RuntimeEvent::LogLine { id, stream: LogStream::System, text });
                        }
                    }
                }
            }
        }
    }
}
