use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc};

#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;

use orkesy_core::config::{RestartPolicy, ServiceConfig};
use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::{RuntimeGraph, ServiceId, ServiceStatus};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::LogStream;

struct ProcessHandle {
    child: Child,
    pgid: i32,
}

struct RestartTracker {
    count: u32,
    window_start: Instant,
}

impl RestartTracker {
    fn new() -> Self {
        Self {
            count: 0,
            window_start: Instant::now(),
        }
    }

    fn can_restart(&mut self, max_restarts: u32, window_secs: u64) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start).as_secs() >= window_secs {
            self.count = 0;
            self.window_start = now;
        }
        if self.count < max_restarts {
            self.count += 1;
            true
        } else {
            false
        }
    }
}

pub struct LocalProcessEngine {
    configs: BTreeMap<ServiceId, ServiceConfig>,
    processes: BTreeMap<ServiceId, ProcessHandle>,
    next_id: Arc<AtomicU64>,
    restart_trackers: BTreeMap<ServiceId, RestartTracker>,
}

impl LocalProcessEngine {
    pub fn new() -> Self {
        Self {
            configs: BTreeMap::new(),
            processes: BTreeMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
            restart_trackers: BTreeMap::new(),
        }
    }

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

        let mut cmd = Command::new(&config.command[0]);
        cmd.args(&config.command[1..]);

        if let Some(cwd) = &config.cwd {
            cmd.current_dir(cwd);
        }

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| e.to_string())?;

        let pgid = child.id().map(|pid| pid as i32).unwrap_or(-1);

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

    async fn kill_service(&mut self, id: &ServiceId, graceful: bool) -> Result<(), String> {
        if let Some(mut handle) = self.processes.remove(id) {
            #[cfg(unix)]
            {
                if handle.pgid > 0 {
                    unsafe {
                        libc::killpg(handle.pgid, libc::SIGTERM);
                    }

                    if graceful {
                        tokio::time::sleep(Duration::from_millis(500)).await;

                        if handle.child.try_wait().ok().flatten().is_none() {
                            unsafe {
                                libc::killpg(handle.pgid, libc::SIGKILL);
                            }
                        }
                    } else {
                        unsafe {
                            libc::killpg(handle.pgid, libc::SIGKILL);
                        }
                    }
                } else {
                    let _ = handle.child.kill().await;
                }
            }

            #[cfg(windows)]
            {
                let _ = graceful; // Suppress unused warning
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
        self.emit(
            &event_tx,
            RuntimeEvent::TopologyLoaded {
                graph: graph.clone(),
            },
        );

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

        let mut check_interval = tokio::time::interval(Duration::from_millis(100));
        check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = check_interval.tick() => {
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
                                id: id.clone(),
                                stream: LogStream::System,
                                text: format!("process exited with code: {:?}", code),
                            },
                        );

                        if let Some(config) = self.configs.get(&id) {
                            let should_restart = match &config.restart {
                                RestartPolicy::Always => true,
                                RestartPolicy::OnFailure => code != Some(0),
                                RestartPolicy::Never => false,
                            };

                            if should_restart {
                                let tracker = self.restart_trackers
                                    .entry(id.clone())
                                    .or_insert_with(RestartTracker::new);

                                let can_restart = tracker.can_restart(3, 60);
                                let restart_count = tracker.count;
                                let delay_ms = config.restart_delay_ms.unwrap_or(1000);

                                if can_restart {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id: id.clone(),
                                            stream: LogStream::System,
                                            text: format!("restarting in {}ms (attempt {}/3)...", delay_ms, restart_count),
                                        },
                                    );

                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;

                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::StatusChanged {
                                            id: id.clone(),
                                            status: ServiceStatus::Restarting,
                                        },
                                    );

                                    match self.spawn_service(&id, &event_tx).await {
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
                                                    text: format!("[error] restart failed: {}", e),
                                                },
                                            );
                                        }
                                    }
                                } else {
                                    self.emit(
                                        &event_tx,
                                        RuntimeEvent::LogLine {
                                            id: id.clone(),
                                            stream: LogStream::System,
                                            text: "restart limit reached (3 restarts in 60s), not restarting".into(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }

                cmd = command_rx.recv() => {
                    let Some(cmd) = cmd else { break };

                    match cmd {
                        EngineCommand::Shutdown => {
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

                            let _ = self.kill_service(&id, true).await;
                            tokio::time::sleep(Duration::from_millis(100)).await;

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

                            let output = tokio::process::Command::new(&cmd[0])
                                .args(&cmd[1..])
                                .output()
                                .await;

                            match output {
                                Ok(out) => {
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
