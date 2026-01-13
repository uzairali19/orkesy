#![cfg(feature = "docker")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc};

use orkesy_core::adapter::{Adapter, AdapterCommand, AdapterEvent, LogStream};
use orkesy_core::unit::{Unit, UnitId, UnitKind, UnitMetrics, UnitStatus};

pub struct DockerAdapter {
    client: Option<Docker>,
    units: BTreeMap<UnitId, Unit>,
    containers: BTreeMap<UnitId, String>,
    next_id: Arc<AtomicU64>,
}

impl DockerAdapter {
    pub fn new() -> Self {
        Self {
            client: None,
            units: BTreeMap::new(),
            containers: BTreeMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn emit(&self, event_tx: &broadcast::Sender<AdapterEvent>, event: AdapterEvent) {
        let _ = event_tx.send(event);
    }

    fn emit_log(&self, event_tx: &broadcast::Sender<AdapterEvent>, id: &str, text: String) {
        self.emit(
            event_tx,
            AdapterEvent::LogLine {
                id: id.to_string(),
                stream: LogStream::System,
                text,
            },
        );
    }

    fn emit_status(
        &self,
        event_tx: &broadcast::Sender<AdapterEvent>,
        id: &str,
        status: UnitStatus,
    ) {
        self.emit(
            event_tx,
            AdapterEvent::StatusChanged {
                id: id.to_string(),
                status,
            },
        );
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

    /// Start a container for a unit
    async fn start_container(
        &mut self,
        id: &UnitId,
        event_tx: &broadcast::Sender<AdapterEvent>,
    ) -> Result<(), String> {
        let client = self.client.as_ref().ok_or("Docker not connected")?;
        let unit = self
            .units
            .get(id)
            .ok_or_else(|| format!("unit not found: {}", id))?;

        // For docker units, the start command is typically "docker compose up" or similar
        // We'll run it as a shell command rather than creating a container directly
        // This is more flexible and matches how users typically define docker units

        // Container name
        let container_name = format!("orkesy-{}", id);

        // If start command looks like a docker command, run it via shell
        if unit.start.starts_with("docker ") {
            self.emit_log(event_tx, id, format!("$ {}", unit.start));

            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&unit.start)
                .output()
                .await
                .map_err(|e| e.to_string())?;

            if !output.stdout.is_empty() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    for line in s.lines() {
                        self.emit(
                            event_tx,
                            AdapterEvent::LogLine {
                                id: id.clone(),
                                stream: LogStream::Stdout,
                                text: line.to_string(),
                            },
                        );
                    }
                }
            }

            if !output.stderr.is_empty() {
                if let Ok(s) = String::from_utf8(output.stderr) {
                    for line in s.lines() {
                        self.emit(
                            event_tx,
                            AdapterEvent::LogLine {
                                id: id.clone(),
                                stream: LogStream::Stderr,
                                text: line.to_string(),
                            },
                        );
                    }
                }
            }

            if !output.status.success() {
                return Err(format!("docker command failed: {:?}", output.status.code()));
            }

            // Track that this unit is "running"
            self.containers.insert(id.clone(), container_name);

            // If there's a logs command, spawn a task to stream logs
            if let Some(logs_cmd) = &unit.logs {
                let tx = event_tx.clone();
                let unit_id = id.clone();
                let logs_cmd = logs_cmd.clone();
                let next_id = self.next_id.clone();

                tokio::spawn(async move {
                    let mut child = match tokio::process::Command::new("sh")
                        .arg("-c")
                        .arg(&logs_cmd)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(c) => c,
                        Err(_) => return,
                    };

                    if let Some(stdout) = child.stdout.take() {
                        let tx = tx.clone();
                        let unit_id = unit_id.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, BufReader};
                            let reader = BufReader::new(stdout);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                let _ = tx.send(AdapterEvent::LogLine {
                                    id: unit_id.clone(),
                                    stream: LogStream::Stdout,
                                    text: line,
                                });
                            }
                        });
                    }

                    // Wait for the logs process (it might run indefinitely)
                    let _ = child.wait().await;
                });
            }

            return Ok(());
        }

        // Otherwise, create an actual Docker container
        // Parse the start command to get image and command
        let parts: Vec<&str> = unit.start.split_whitespace().collect();
        let image = parts.first().copied().unwrap_or("busybox");
        let cmd: Vec<&str> = if parts.len() > 1 {
            parts[1..].to_vec()
        } else {
            vec![]
        };

        let env: Vec<String> = unit
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        let container_config = Config {
            image: Some(image),
            cmd: if cmd.is_empty() { None } else { Some(cmd) },
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
                        let _ = log_tx.send(AdapterEvent::LogLine {
                            id: service_id.clone(),
                            stream: LogStream::Stdout,
                            text,
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(())
    }

    /// Stop a container
    async fn stop_container(&mut self, id: &UnitId) -> Result<(), String> {
        let unit = self.units.get(id);

        // If unit has a custom stop command, use it
        if let Some(unit) = unit {
            if let orkesy_core::unit::StopBehavior::Command(cmd) = &unit.stop {
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .await
                    .map_err(|e| e.to_string())?;

                self.containers.remove(id);

                if !output.status.success() {
                    return Err(format!("stop command failed: {:?}", output.status.code()));
                }
                return Ok(());
            }
        }

        // Otherwise try to stop via Docker API
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

impl Default for DockerAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for DockerAdapter {
    fn name(&self) -> &'static str {
        "docker"
    }

    async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<AdapterCommand>,
        event_tx: broadcast::Sender<AdapterEvent>,
        units: Vec<Unit>,
    ) {
        // Only handle docker units
        for unit in units {
            if unit.kind == UnitKind::Docker {
                self.units.insert(unit.id.clone(), unit);
            }
        }

        if self.units.is_empty() {
            return; // No docker units to manage
        }

        // Connect to Docker
        if let Err(e) = self.connect().await {
            self.emit_log(&event_tx, "docker", format!("[error] {}", e));
            return;
        }

        self.emit_log(&event_tx, "docker", "connected to Docker".into());

        // Main command loop
        while let Some(cmd) = command_rx.recv().await {
            match cmd {
                AdapterCommand::Shutdown => {
                    // Stop all containers
                    let ids: Vec<_> = self.containers.keys().cloned().collect();
                    for id in ids {
                        let _ = self.stop_container(&id).await;
                    }
                    break;
                }

                AdapterCommand::Start { id } => {
                    if !self.units.contains_key(&id) {
                        continue; // Not a docker unit
                    }

                    if self.containers.contains_key(&id) {
                        self.emit_log(&event_tx, &id, "[warn] already running".into());
                        continue;
                    }

                    self.emit_status(&event_tx, &id, UnitStatus::Starting);

                    match self.start_container(&id, &event_tx).await {
                        Ok(()) => {
                            self.emit_status(&event_tx, &id, UnitStatus::Running);
                        }
                        Err(e) => {
                            self.emit_status(
                                &event_tx,
                                &id,
                                UnitStatus::Errored { message: e.clone() },
                            );
                            self.emit_log(&event_tx, &id, format!("[error] {}", e));
                        }
                    }
                }

                AdapterCommand::Stop { id } => {
                    if !self.units.contains_key(&id) {
                        continue;
                    }

                    self.emit_log(&event_tx, &id, "stopping...".into());
                    self.emit_status(&event_tx, &id, UnitStatus::Stopping);

                    match self.stop_container(&id).await {
                        Ok(()) => {
                            self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                        }
                        Err(e) => {
                            self.emit_log(&event_tx, &id, format!("[warn] {}", e));
                        }
                    }
                }

                AdapterCommand::Restart { id } => {
                    if !self.units.contains_key(&id) {
                        continue;
                    }

                    self.emit_log(&event_tx, &id, "restarting...".into());

                    let _ = self.stop_container(&id).await;
                    tokio::time::sleep(Duration::from_millis(100)).await;

                    self.emit_status(&event_tx, &id, UnitStatus::Starting);
                    match self.start_container(&id, &event_tx).await {
                        Ok(()) => {
                            self.emit_status(&event_tx, &id, UnitStatus::Running);
                            self.emit_log(&event_tx, &id, "restarted".into());
                        }
                        Err(e) => {
                            self.emit_status(
                                &event_tx,
                                &id,
                                UnitStatus::Errored { message: e.clone() },
                            );
                            self.emit_log(&event_tx, &id, format!("[error] restart failed: {}", e));
                        }
                    }
                }

                AdapterCommand::Kill { id } => {
                    if !self.units.contains_key(&id) {
                        continue;
                    }

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
                    self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                    self.emit_log(&event_tx, &id, "killed".into());
                }

                AdapterCommand::Toggle { id } => {
                    if !self.units.contains_key(&id) {
                        continue;
                    }

                    if self.containers.contains_key(&id) {
                        self.emit_log(&event_tx, &id, "stopping...".into());
                        self.emit_status(&event_tx, &id, UnitStatus::Stopping);
                        let _ = self.stop_container(&id).await;
                        self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                    } else {
                        self.emit_status(&event_tx, &id, UnitStatus::Starting);
                        match self.start_container(&id, &event_tx).await {
                            Ok(()) => {
                                self.emit_status(&event_tx, &id, UnitStatus::Running);
                            }
                            Err(e) => {
                                self.emit_status(
                                    &event_tx,
                                    &id,
                                    UnitStatus::Errored { message: e.clone() },
                                );
                                self.emit_log(&event_tx, &id, format!("[error] {}", e));
                            }
                        }
                    }
                }

                AdapterCommand::ClearLogs { id } => {
                    if self.units.contains_key(&id) {
                        self.emit_log(&event_tx, &id, "logs cleared".into());
                    }
                }

                AdapterCommand::Install { id } => {
                    // Docker units typically don't have install steps
                    if self.units.contains_key(&id) {
                        self.emit_log(&event_tx, &id, "docker units have no install step".into());
                    }
                }

                AdapterCommand::Exec { id, cmd } => {
                    if !self.units.contains_key(&id) {
                        continue;
                    }

                    let shown = cmd.join(" ");
                    self.emit_log(&event_tx, &id, format!("$ {}", shown));

                    // For docker compose style, exec won't work easily
                    // For now just log
                    self.emit_log(
                        &event_tx,
                        &id,
                        "(docker exec not yet implemented for compose-style units)".into(),
                    );
                }
            }
        }
    }

    fn status(&self, id: &str) -> Option<UnitStatus> {
        if self.containers.contains_key(id) {
            Some(UnitStatus::Running)
        } else if self.units.contains_key(id) {
            Some(UnitStatus::Stopped)
        } else {
            None
        }
    }

    fn metrics(&self, _id: &str) -> Option<UnitMetrics> {
        // Docker metrics would require docker stats API
        None
    }
}
