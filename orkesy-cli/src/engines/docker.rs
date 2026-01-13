#![cfg(feature = "docker")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc};

use orkesy_core::config::ServiceConfig;
use orkesy_core::engine::{Engine, EngineCommand};
use orkesy_core::model::{RuntimeGraph, ServiceId, ServiceStatus};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};

pub struct DockerEngine {
    client: Option<Docker>,
    configs: BTreeMap<ServiceId, ServiceConfig>,
    containers: BTreeMap<ServiceId, String>,
    next_id: Arc<AtomicU64>,
}

impl DockerEngine {
    pub fn new() -> Self {
        Self {
            client: None,
            configs: BTreeMap::new(),
            containers: BTreeMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
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

    async fn connect(&mut self) -> Result<(), String> {
        if self.client.is_some() {
            return Ok(());
        }

        let client = Docker::connect_with_local_defaults()
            .map_err(|e| format!("failed to connect to Docker: {}", e))?;

        // Verify connection
        client
            .ping()
            .await
            .map_err(|e| format!("Docker ping failed: {}", e))?;

        self.client = Some(client);
        Ok(())
    }

    async fn start_container(
        &mut self,
        id: &ServiceId,
        event_tx: &broadcast::Sender<EventEnvelope>,
    ) -> Result<(), String> {
        let client = self.client.as_ref().ok_or("Docker not connected")?;
        let config = self
            .configs
            .get(id)
            .ok_or_else(|| format!("no config for service: {}", id))?;

        // Container name
        let container_name = format!("orkesy-{}", id);

        // Build container config
        let cmd: Vec<&str> = config.command.iter().map(|s| s.as_str()).collect();

        let env: Vec<String> = config
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        let container_config = Config {
            image: Some(
                config
                    .command
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("busybox"),
            ),
            cmd: if cmd.len() > 1 {
                Some(cmd[1..].to_vec())
            } else {
                None
            },
            env: Some(env.iter().map(|s| s.as_str()).collect()),
            ..Default::default()
        };

        // Create container
        let create_options = CreateContainerOptions {
            name: container_name.clone(),
            platform: None,
        };

        let response = client
            .create_container(Some(create_options), container_config)
            .await
            .map_err(|e| format!("failed to create container: {}", e))?;

        let container_id = response.id;
        self.containers.insert(id.clone(), container_id.clone());

        // Start container
        client
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| format!("failed to start container: {}", e))?;

        // Spawn log streaming task
        let log_tx = event_tx.clone();
        let service_id = id.clone();
        let log_client = client.clone();
        let log_container_id = container_id.clone();
        let next_id = self.next_id.clone();

        tokio::spawn(async move {
            let options = LogsOptions::<String> {
                follow: true,
                stdout: true,
                stderr: true,
                ..Default::default()
            };

            let mut stream = log_client.logs(&log_container_id, Some(options));

            while let Some(result) = stream.next().await {
                match result {
                    Ok(log) => {
                        let text = log.to_string();
                        let _ = log_tx.send(EventEnvelope {
                            id: next_id.fetch_add(1, Ordering::SeqCst),
                            at: SystemTime::now(),
                            event: RuntimeEvent::LogLine {
                                id: service_id.clone(),
                                text,
                            },
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(())
    }

    async fn stop_container(&mut self, id: &ServiceId) -> Result<(), String> {
        let client = self.client.as_ref().ok_or("Docker not connected")?;

        if let Some(container_id) = self.containers.remove(id) {
            let options = StopContainerOptions { t: 10 };

            client
                .stop_container(&container_id, Some(options))
                .await
                .map_err(|e| format!("failed to stop container: {}", e))?;

            // Remove container
            let remove_options = RemoveContainerOptions {
                force: true,
                ..Default::default()
            };

            client
                .remove_container(&container_id, Some(remove_options))
                .await
                .ok(); // Ignore removal errors
        }

        Ok(())
    }
}

impl Default for DockerEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Engine for DockerEngine {
    fn name(&self) -> &'static str {
        "docker"
    }

    async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<EngineCommand>,
        event_tx: broadcast::Sender<EventEnvelope>,
        graph: RuntimeGraph,
    ) {
        // Connect to Docker
        if let Err(e) = self.connect().await {
            self.emit(
                &event_tx,
                RuntimeEvent::LogLine {
                    id: "docker".into(),
                    text: format!("[error] {}", e),
                },
            );
            return;
        }

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

                match self.start_container(id, &event_tx).await {
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
                                text: format!("[error] {}", e),
                            },
                        );
                    }
                }
            }
        }

        // Main command loop
        while let Some(cmd) = command_rx.recv().await {
            match cmd {
                EngineCommand::Shutdown => {
                    // Stop all containers
                    let ids: Vec<_> = self.containers.keys().cloned().collect();
                    for id in ids {
                        let _ = self.stop_container(&id).await;
                    }
                    break;
                }

                EngineCommand::Start { id } => {
                    self.emit(
                        &event_tx,
                        RuntimeEvent::StatusChanged {
                            id: id.clone(),
                            status: ServiceStatus::Starting,
                        },
                    );

                    match self.start_container(&id, &event_tx).await {
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
                                    text: format!("[error] {}", e),
                                },
                            );
                        }
                    }
                }

                EngineCommand::Stop { id } => match self.stop_container(&id).await {
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
                                text: format!("[warn] {}", e),
                            },
                        );
                    }
                },

                EngineCommand::Restart { id } => {
                    self.emit(
                        &event_tx,
                        RuntimeEvent::StatusChanged {
                            id: id.clone(),
                            status: ServiceStatus::Restarting,
                        },
                    );

                    let _ = self.stop_container(&id).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;

                    match self.start_container(&id, &event_tx).await {
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
                                    text: format!("[error] restart failed: {}", e),
                                },
                            );
                        }
                    }
                }

                EngineCommand::Kill { id } => {
                    // Force remove container
                    if let Some(container_id) = self.containers.remove(&id) {
                        if let Some(client) = &self.client {
                            let options = RemoveContainerOptions {
                                force: true,
                                ..Default::default()
                            };
                            let _ = client.remove_container(&container_id, Some(options)).await;
                        }
                    }
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
                            text: "killed".into(),
                        },
                    );
                }

                EngineCommand::Toggle { id } => {
                    if self.containers.contains_key(&id) {
                        // Running -> Stop
                        let _ = self.stop_container(&id).await;
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
                        match self.start_container(&id, &event_tx).await {
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
                    // Docker exec is more complex - for now just log the command
                    let shown = cmd.join(" ");
                    self.emit(
                        &event_tx,
                        RuntimeEvent::LogLine {
                            id: id.clone(),
                            text: format!("$ {} (docker exec not yet implemented)", shown),
                        },
                    );
                }

                EngineCommand::EmitLog { id, text } => {
                    self.emit(&event_tx, RuntimeEvent::LogLine { id, text });
                }
            }
        }
    }
}
