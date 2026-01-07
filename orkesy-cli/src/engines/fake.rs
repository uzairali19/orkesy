use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::{RuntimeGraph, ServiceId, ServiceStatus};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};

/// A fake engine that simulates service lifecycle for demos and testing.
///
/// This engine:
/// - Simulates Starting -> Running transitions with delays
/// - Auto-streams fake log messages for running services
/// - Responds to all commands but doesn't actually run any processes
pub struct FakeEngine {
    /// Interval between auto-generated log messages
    tick_interval: Duration,
}

impl FakeEngine {
    pub fn new() -> Self {
        Self {
            tick_interval: Duration::from_millis(600),
        }
    }

    pub fn with_tick_interval(mut self, interval: Duration) -> Self {
        self.tick_interval = interval;
        self
    }
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for FakeEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<EngineCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
        graph: RuntimeGraph,
    ) {
        let mut next_id: u64 = 1;

        // Helper to emit events
        let mut emit = |event: RuntimeEvent| {
            let _ = event_tx.send(EventEnvelope {
                id: next_id,
                at: SystemTime::now(),
                event,
            });
            next_id += 1;
        };

        // Track status locally (engine source of truth for streaming decision)
        let mut statuses: BTreeMap<ServiceId, ServiceStatus> = graph
            .nodes
            .keys()
            .map(|id| (id.clone(), ServiceStatus::Stopped))
            .collect();

        // Boot: topology loaded
        emit(RuntimeEvent::TopologyLoaded { graph: graph.clone() });

        // Auto-start services marked with autostart
        for (id, node) in &graph.nodes {
            if matches!(node.desired, orkesy_core::model::DesiredState::Running) {
                emit(RuntimeEvent::StatusChanged {
                    id: id.clone(),
                    status: ServiceStatus::Starting,
                });
                statuses.insert(id.clone(), ServiceStatus::Starting);
            }
        }

        // Give services time to "start"
        tokio::time::sleep(Duration::from_millis(250)).await;

        // Mark starting services as running
        for (id, status) in statuses.iter_mut() {
            if matches!(status, ServiceStatus::Starting) {
                let _ = event_tx.send(EventEnvelope {
                    id: next_id,
                    at: SystemTime::now(),
                    event: RuntimeEvent::StatusChanged {
                        id: id.clone(),
                        status: ServiceStatus::Running,
                    },
                });
                next_id += 1;
                *status = ServiceStatus::Running;

                // Emit a startup log
                let _ = event_tx.send(EventEnvelope {
                    id: next_id,
                    at: SystemTime::now(),
                    event: RuntimeEvent::LogLine {
                        id: id.clone(),
                        text: format!("{} started", id),
                    },
                });
                next_id += 1;
            }
        }

        // Auto-streaming logs
        let mut tick = tokio::time::interval(self.tick_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    for (id, st) in statuses.iter() {
                        if matches!(st, ServiceStatus::Running) {
                            let text = match id.as_str() {
                                "api" => "GET /health 200",
                                "worker" => "processed job id=42",
                                "postgres" | "db" => "checkpoint complete",
                                "redis" | "cache" => "keys: 1024, memory: 2.1MB",
                                _ => "tick",
                            };
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id: id.clone(),
                                    text: text.into(),
                                },
                            });
                            next_id += 1;
                        }
                    }
                }

                maybe_cmd = command_rx.recv() => {
                    let Some(cmd) = maybe_cmd else { break; };

                    match cmd {
                        EngineCommand::Shutdown => break,

                        EngineCommand::Start { id } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Starting,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id.clone(), ServiceStatus::Starting);

                            tokio::time::sleep(Duration::from_millis(300)).await;

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Running,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id, ServiceStatus::Running);
                        }

                        EngineCommand::Stop { id } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Stopped,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id, ServiceStatus::Stopped);
                        }

                        EngineCommand::Restart { id } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Restarting,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id.clone(), ServiceStatus::Restarting);

                            tokio::time::sleep(Duration::from_millis(200)).await;

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Running,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id.clone(), ServiceStatus::Running);

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id,
                                    text: "restarted".into(),
                                },
                            });
                            next_id += 1;
                        }

                        EngineCommand::Kill { id } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::StatusChanged {
                                    id: id.clone(),
                                    status: ServiceStatus::Stopped,
                                },
                            });
                            next_id += 1;
                            statuses.insert(id.clone(), ServiceStatus::Stopped);

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id,
                                    text: "process killed".into(),
                                },
                            });
                            next_id += 1;
                        }

                        EngineCommand::Toggle { id } => {
                            let st = statuses.get(&id).cloned().unwrap_or(ServiceStatus::Unknown);
                            if matches!(
                                st,
                                ServiceStatus::Running
                                    | ServiceStatus::Starting
                                    | ServiceStatus::Restarting
                            ) {
                                let _ = event_tx.send(EventEnvelope {
                                    id: next_id,
                                    at: SystemTime::now(),
                                    event: RuntimeEvent::StatusChanged {
                                        id: id.clone(),
                                        status: ServiceStatus::Stopped,
                                    },
                                });
                                next_id += 1;
                                statuses.insert(id, ServiceStatus::Stopped);
                            } else {
                                let _ = event_tx.send(EventEnvelope {
                                    id: next_id,
                                    at: SystemTime::now(),
                                    event: RuntimeEvent::StatusChanged {
                                        id: id.clone(),
                                        status: ServiceStatus::Starting,
                                    },
                                });
                                next_id += 1;
                                statuses.insert(id.clone(), ServiceStatus::Starting);

                                tokio::time::sleep(Duration::from_millis(250)).await;

                                let _ = event_tx.send(EventEnvelope {
                                    id: next_id,
                                    at: SystemTime::now(),
                                    event: RuntimeEvent::StatusChanged {
                                        id: id.clone(),
                                        status: ServiceStatus::Running,
                                    },
                                });
                                next_id += 1;
                                statuses.insert(id, ServiceStatus::Running);
                            }
                        }

                        EngineCommand::ClearLogs { id } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::ClearLogs { id },
                            });
                            next_id += 1;
                        }

                        EngineCommand::Exec { id, cmd } => {
                            let shown = cmd.join(" ");
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id: id.clone(),
                                    text: format!("$ {shown}"),
                                },
                            });
                            next_id += 1;

                            tokio::time::sleep(Duration::from_millis(120)).await;

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id,
                                    text: "ok".into(),
                                },
                            });
                            next_id += 1;
                        }

                        EngineCommand::EmitLog { id, text } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine { id, text },
                            });
                            next_id += 1;
                        }
                    }
                }
            }
        }
    }
}
