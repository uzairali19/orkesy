use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::{RuntimeGraph, ServiceId, ServiceStatus};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::LogStream;
use orkesy_core::unit::UnitMetrics;

pub struct FakeEngine {
    tick_interval: Duration,
    tick_counter: u64,
    job_counter: u64,
}

impl FakeEngine {
    pub fn new() -> Self {
        Self {
            tick_interval: Duration::from_millis(600),
            tick_counter: 0,
            job_counter: 0,
        }
    }

    #[allow(dead_code)]
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

        let mut emit = |event: RuntimeEvent| {
            let _ = event_tx.send(EventEnvelope {
                id: next_id,
                at: SystemTime::now(),
                event,
            });
            next_id += 1;
        };

        let mut statuses: BTreeMap<ServiceId, ServiceStatus> = graph
            .nodes
            .keys()
            .map(|id| (id.clone(), ServiceStatus::Stopped))
            .collect();

        emit(RuntimeEvent::TopologyLoaded {
            graph: graph.clone(),
        });

        for (id, node) in &graph.nodes {
            if matches!(node.desired, orkesy_core::model::DesiredState::Running) {
                emit(RuntimeEvent::StatusChanged {
                    id: id.clone(),
                    status: ServiceStatus::Starting,
                });
                statuses.insert(id.clone(), ServiceStatus::Starting);
            }
        }

        tokio::time::sleep(Duration::from_millis(250)).await;

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

                let _ = event_tx.send(EventEnvelope {
                    id: next_id,
                    at: SystemTime::now(),
                    event: RuntimeEvent::LogLine {
                        id: id.clone(),
                        stream: LogStream::System,
                        text: format!("{} started", id),
                    },
                });
                next_id += 1;
            }
        }

        let mut tick = tokio::time::interval(self.tick_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    self.tick_counter += 1;
                    let tick_num = self.tick_counter;

                    for (id, st) in statuses.iter() {
                        if matches!(st, ServiceStatus::Running) {
                            let text: String = match id.as_str() {
                                "api" => {
                                    if tick_num % 12 == 7 {
                                        "[ERROR] Connection refused to upstream service".into()
                                    } else if tick_num % 8 == 3 {
                                        "[WARN] High latency detected: 450ms".into()
                                    } else {
                                        let routes = ["GET /health 200", "GET /api/users 200", "POST /api/data 201", "GET /api/status 200"];
                                        routes[(tick_num as usize) % routes.len()].into()
                                    }
                                }
                                "worker" => {
                                    self.job_counter += 1;
                                    if tick_num % 10 == 5 {
                                        format!("[ERROR] Job {} failed: timeout after 30s", self.job_counter)
                                    } else if tick_num % 7 == 2 {
                                        format!("[WARN] Queue depth high: {} pending", 50 + (tick_num % 30))
                                    } else {
                                        format!("processed job id={}", self.job_counter)
                                    }
                                }
                                "postgres" | "db" => {
                                    if tick_num % 15 == 10 {
                                        "[WARN] Slow query detected: 1250ms".into()
                                    } else {
                                        let msgs = ["checkpoint complete", "autovacuum: processing", "connection accepted"];
                                        msgs[(tick_num as usize) % msgs.len()].into()
                                    }
                                }
                                "redis" | "cache" => {
                                    let keys = 1000 + (tick_num % 500);
                                    let mem = 2.0 + (tick_num % 10) as f32 * 0.1;
                                    format!("keys: {}, memory: {:.1}MB", keys, mem)
                                }
                                _ => format!("tick {}", tick_num),
                            };

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine {
                                    id: id.clone(),
                                    stream: LogStream::Stdout,
                                    text,
                                },
                            });
                            next_id += 1;

                            let (base_cpu, base_mem) = match id.as_str() {
                                "api" => (12.0, 150_000_000u64),
                                "worker" => (35.0, 280_000_000u64),
                                "postgres" | "db" => (8.0, 512_000_000u64),
                                "redis" | "cache" => (3.0, 64_000_000u64),
                                _ => (5.0, 50_000_000u64),
                            };

                            let cpu = base_cpu + ((tick_num % 10) as f32 * 1.5) - 5.0;
                            let memory = base_mem + ((tick_num % 20) * 1_000_000);

                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::MetricsUpdated {
                                    id: id.clone(),
                                    metrics: UnitMetrics {
                                        cpu_percent: cpu.max(0.1),
                                        memory_bytes: memory,
                                        uptime_secs: tick_num * self.tick_interval.as_secs().max(1),
                                        pid: Some(10000 + (id.len() as u32 * 100)),
                                    },
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
                                    stream: LogStream::System,
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
                                    stream: LogStream::System,
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
                                    stream: LogStream::System,
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
                                    stream: LogStream::System,
                                    text: "ok".into(),
                                },
                            });
                            next_id += 1;
                        }

                        EngineCommand::EmitLog { id, text } => {
                            let _ = event_tx.send(EventEnvelope {
                                id: next_id,
                                at: SystemTime::now(),
                                event: RuntimeEvent::LogLine { id, stream: LogStream::System, text },
                            });
                            next_id += 1;
                        }
                    }
                }
            }
        }
    }
}
