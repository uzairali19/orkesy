use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::SystemTime;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use orkesy_core::command::{CommandSpec, RunId};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::LogStream;

#[allow(dead_code)]
#[derive(Debug)]
pub enum RunnerCommand {
    Run { spec: CommandSpec },
    RunArbitrary {
        title: String,
        command: String,
        cwd: Option<std::path::PathBuf>,
    },
    Kill { run_id: RunId },
    Rerun { run_id: RunId },
    Shutdown,
}

struct ProcessHandle {
    child: Child,
    pgid: i32,
}

pub struct CommandRunner {
    processes: BTreeMap<RunId, ProcessHandle>,
    specs: BTreeMap<RunId, CommandSpec>,
    next_event_id: u64,
}

impl CommandRunner {
    pub fn new() -> Self {
        Self {
            processes: BTreeMap::new(),
            specs: BTreeMap::new(),
            // Start high to avoid collision with adapter events
            next_event_id: 1_000_000,
        }
    }

    pub async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<RunnerCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
    ) {
        let mut check_interval = tokio::time::interval(std::time::Duration::from_millis(100));

        loop {
            tokio::select! {
                // Check for exited processes
                _ = check_interval.tick() => {
                    let mut finished = vec![];
                    for (run_id, handle) in &mut self.processes {
                        if let Ok(Some(status)) = handle.child.try_wait() {
                            finished.push((run_id.clone(), status.code()));
                        }
                    }

                    for (run_id, code) in finished {
                        self.processes.remove(&run_id);
                        let _ = event_tx.send(EventEnvelope {
                            id: self.next_event_id,
                            at: SystemTime::now(),
                            event: RuntimeEvent::CommandFinished {
                                run_id,
                                exit_code: code,
                            },
                        });
                        self.next_event_id += 1;
                    }
                }

                // Handle commands
                cmd = command_rx.recv() => {
                    let Some(cmd) = cmd else { break };

                    match cmd {
                        RunnerCommand::Shutdown => {
                            // Kill all running processes
                            let run_ids: Vec<_> = self.processes.keys().cloned().collect();
                            for run_id in run_ids {
                                if let Some(handle) = self.processes.remove(&run_id) {
                                    Self::kill_process(handle).await;
                                }
                            }
                            break;
                        }

                        RunnerCommand::Run { spec } => {
                            let run_id = Uuid::new_v4().to_string();
                            if let Err(e) = self.spawn_command(&spec, &run_id, &event_tx).await {
                                self.emit_error(&event_tx, &run_id, &e);
                            } else {
                                self.specs.insert(run_id, spec);
                            }
                        }

                        RunnerCommand::RunArbitrary { title, command, cwd } => {
                            let run_id = Uuid::new_v4().to_string();
                            let spec = CommandSpec {
                                id: format!("arbitrary:{}", run_id),
                                tool: orkesy_core::command::DetectedTool::Rust, // Placeholder
                                name: title.clone(),
                                display_name: title,
                                command,
                                cwd,
                                description: None,
                                category: orkesy_core::command::CommandCategory::Script,
                            };
                            if let Err(e) = self.spawn_command(&spec, &run_id, &event_tx).await {
                                self.emit_error(&event_tx, &run_id, &e);
                            } else {
                                self.specs.insert(run_id, spec);
                            }
                        }

                        RunnerCommand::Kill { run_id } => {
                            if let Some(handle) = self.processes.remove(&run_id) {
                                Self::kill_process(handle).await;
                                let _ = event_tx.send(EventEnvelope {
                                    id: self.next_event_id,
                                    at: SystemTime::now(),
                                    event: RuntimeEvent::CommandKilled { run_id },
                                });
                                self.next_event_id += 1;
                            }
                        }

                        RunnerCommand::Rerun { run_id } => {
                            if let Some(spec) = self.specs.get(&run_id).cloned() {
                                // Kill existing if still running
                                if let Some(handle) = self.processes.remove(&run_id) {
                                    Self::kill_process(handle).await;
                                }

                                // Start new run with same spec
                                let new_run_id = Uuid::new_v4().to_string();
                                if let Err(e) = self.spawn_command(&spec, &new_run_id, &event_tx).await {
                                    self.emit_error(&event_tx, &new_run_id, &e);
                                } else {
                                    self.specs.insert(new_run_id, spec);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn spawn_command(
        &mut self,
        spec: &CommandSpec,
        run_id: &str,
        event_tx: &broadcast::Sender<EventEnvelope>,
    ) -> Result<(), String> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg(&spec.command);

        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }

        // Create new process group for reliable cleanup
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
        let pid = child.id();

        // Emit CommandStarted
        let _ = event_tx.send(EventEnvelope {
            id: self.next_event_id,
            at: SystemTime::now(),
            event: RuntimeEvent::CommandStarted {
                run_id: run_id.to_string(),
                command_id: spec.id.clone(),
                command: spec.command.clone(),
                display_name: spec.display_name.clone(),
                pid,
            },
        });
        self.next_event_id += 1;

        // Spawn stdout reader
        if let Some(stdout) = child.stdout.take() {
            let tx = event_tx.clone();
            let rid = run_id.to_string();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(EventEnvelope {
                        id: 0, // Will be ordered by receiver
                        at: SystemTime::now(),
                        event: RuntimeEvent::CommandOutput {
                            run_id: rid.clone(),
                            stream: LogStream::Stdout,
                            text: line,
                        },
                    });
                }
            });
        }

        // Spawn stderr reader
        if let Some(stderr) = child.stderr.take() {
            let tx = event_tx.clone();
            let rid = run_id.to_string();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(EventEnvelope {
                        id: 0,
                        at: SystemTime::now(),
                        event: RuntimeEvent::CommandOutput {
                            run_id: rid.clone(),
                            stream: LogStream::Stderr,
                            text: line,
                        },
                    });
                }
            });
        }

        self.processes.insert(
            run_id.to_string(),
            ProcessHandle {
                child,
                pgid,
            },
        );

        Ok(())
    }

    fn emit_error(
        &mut self,
        event_tx: &broadcast::Sender<EventEnvelope>,
        run_id: &str,
        error: &str,
    ) {
        let _ = event_tx.send(EventEnvelope {
            id: self.next_event_id,
            at: SystemTime::now(),
            event: RuntimeEvent::CommandOutput {
                run_id: run_id.to_string(),
                stream: LogStream::System,
                text: format!("[error] {}", error),
            },
        });
        self.next_event_id += 1;
    }

    async fn kill_process(mut handle: ProcessHandle) {
        if handle.pgid > 0 {
            #[cfg(unix)]
            unsafe {
                // Try graceful shutdown first
                libc::killpg(handle.pgid, libc::SIGTERM);
            }

            // Wait a bit for graceful shutdown
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Force kill if still running
            if handle.child.try_wait().ok().flatten().is_none() {
                #[cfg(unix)]
                unsafe {
                    libc::killpg(handle.pgid, libc::SIGKILL);
                }
            }
        } else {
            // Fallback: kill just the child
            let _ = handle.child.kill().await;
        }
    }

    #[allow(dead_code)]
    pub fn is_running(&self, run_id: &RunId) -> bool {
        self.processes.contains_key(run_id)
    }

    #[allow(dead_code)]
    pub fn running_ids(&self) -> Vec<RunId> {
        self.processes.keys().cloned().collect()
    }
}

impl Default for CommandRunner {
    fn default() -> Self {
        Self::new()
    }
}
