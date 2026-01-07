use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::net::TcpStream;
use tokio::sync::broadcast;

use orkesy_core::config::HealthCheck;
use orkesy_core::model::{HealthStatus, ServiceId};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};

/// A background health checker for a single service
pub struct HealthChecker {
    service_id: ServiceId,
    check: HealthCheck,
    port: Option<u16>,
    next_id: Arc<AtomicU64>,
}

impl HealthChecker {
    pub fn new(
        service_id: ServiceId,
        check: HealthCheck,
        port: Option<u16>,
        next_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            service_id,
            check,
            port,
            next_id,
        }
    }

    /// Run the health checker in a loop
    pub async fn run(self, event_tx: broadcast::Sender<EventEnvelope>) {
        let interval = match &self.check {
            HealthCheck::Http { interval_ms, .. } => Duration::from_millis(*interval_ms),
            HealthCheck::Tcp { interval_ms } => Duration::from_millis(*interval_ms),
            HealthCheck::Exec { interval_ms, .. } => Duration::from_millis(*interval_ms),
        };

        let mut ticker = tokio::time::interval(interval);

        loop {
            ticker.tick().await;

            let health = match &self.check {
                HealthCheck::Http {
                    path, timeout_ms, ..
                } => self.check_http(path, Duration::from_millis(*timeout_ms)).await,
                HealthCheck::Tcp { .. } => self.check_tcp().await,
                HealthCheck::Exec { command, .. } => self.check_exec(command).await,
            };

            let _ = event_tx.send(EventEnvelope {
                id: self.next_id.fetch_add(1, Ordering::SeqCst),
                at: SystemTime::now(),
                event: RuntimeEvent::HealthChanged {
                    id: self.service_id.clone(),
                    health,
                },
            });
        }
    }

    /// Check health via TCP connection
    async fn check_tcp(&self) -> HealthStatus {
        let port = self.port.unwrap_or(80);
        let addr = format!("127.0.0.1:{}", port);

        match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => HealthStatus::Healthy,
            Ok(Err(e)) => HealthStatus::Unhealthy {
                reason: e.to_string(),
            },
            Err(_) => HealthStatus::Unhealthy {
                reason: "connection timeout".into(),
            },
        }
    }

    /// Check health via HTTP GET request (requires reqwest feature)
    #[cfg(feature = "health-http")]
    async fn check_http(&self, path: &str, timeout: Duration) -> HealthStatus {
        let port = self.port.unwrap_or(80);
        let url = format!("http://127.0.0.1:{}{}", port, path);

        match tokio::time::timeout(timeout, reqwest::get(&url)).await {
            Ok(Ok(resp)) if resp.status().is_success() => HealthStatus::Healthy,
            Ok(Ok(resp)) => HealthStatus::Degraded {
                reason: format!("HTTP {}", resp.status()),
            },
            Ok(Err(e)) => HealthStatus::Unhealthy {
                reason: e.to_string(),
            },
            Err(_) => HealthStatus::Unhealthy {
                reason: "timeout".into(),
            },
        }
    }

    /// Fallback HTTP check when reqwest is not available
    #[cfg(not(feature = "health-http"))]
    async fn check_http(&self, _path: &str, _timeout: Duration) -> HealthStatus {
        // Fall back to TCP check when HTTP client is not available
        self.check_tcp().await
    }

    /// Check health via command execution
    async fn check_exec(&self, command: &[String]) -> HealthStatus {
        if command.is_empty() {
            return HealthStatus::Unknown;
        }

        let output = tokio::process::Command::new(&command[0])
            .args(&command[1..])
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => HealthStatus::Healthy,
            Ok(o) => HealthStatus::Unhealthy {
                reason: format!("exit code: {:?}", o.status.code()),
            },
            Err(e) => HealthStatus::Unhealthy {
                reason: e.to_string(),
            },
        }
    }
}

/// Spawn health checkers for all services that have health checks configured
pub fn spawn_health_checkers(
    configs: &std::collections::BTreeMap<String, orkesy_core::config::ServiceConfig>,
    event_tx: broadcast::Sender<EventEnvelope>,
    next_id: Arc<AtomicU64>,
) {
    for (id, config) in configs {
        if let Some(check) = &config.health_check {
            let checker = HealthChecker::new(
                id.clone(),
                check.clone(),
                config.port,
                next_id.clone(),
            );
            let tx = event_tx.clone();
            tokio::spawn(async move {
                checker.run(tx).await;
            });
        }
    }
}
