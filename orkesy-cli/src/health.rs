use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use tokio::net::TcpStream;
use tokio::sync::broadcast;

use orkesy_core::model::{HealthStatus, ServiceId};
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::unit::{HealthCheck, Unit};

/// A background health checker for a single service
#[allow(dead_code)]
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
            HealthCheck::Tcp { interval_ms, .. } => Duration::from_millis(*interval_ms),
            HealthCheck::Exec { interval_ms, .. } => Duration::from_millis(*interval_ms),
        };

        let mut ticker = tokio::time::interval(interval);

        loop {
            ticker.tick().await;

            let health = match &self.check {
                HealthCheck::Http {
                    url, timeout_ms, ..
                } => {
                    self.check_http_url(url, Duration::from_millis(*timeout_ms))
                        .await
                }
                HealthCheck::Tcp { port, .. } => self.check_tcp_port(*port).await,
                HealthCheck::Exec { command, .. } => self.check_exec_str(command).await,
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

    /// Check health via TCP connection to specific port
    async fn check_tcp_port(&self, port: u16) -> HealthStatus {
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

    /// Check health via HTTP GET request to URL (requires reqwest feature)
    #[cfg(feature = "health-http")]
    async fn check_http_url(&self, url: &str, timeout: Duration) -> HealthStatus {
        match tokio::time::timeout(timeout, reqwest::get(url)).await {
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

    /// Fallback HTTP check when reqwest is not available - just try TCP connect
    #[cfg(not(feature = "health-http"))]
    async fn check_http_url(&self, url: &str, _timeout: Duration) -> HealthStatus {
        // Parse URL to get host and port, then do TCP check
        // Simple parsing for http://host:port/...
        let url = url
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let host_port = url.split('/').next().unwrap_or("127.0.0.1:80");

        match tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(host_port)).await {
            Ok(Ok(_)) => HealthStatus::Healthy,
            Ok(Err(e)) => HealthStatus::Unhealthy {
                reason: e.to_string(),
            },
            Err(_) => HealthStatus::Unhealthy {
                reason: "connection timeout".into(),
            },
        }
    }

    /// Check health via command execution (shell string)
    async fn check_exec_str(&self, command: &str) -> HealthStatus {
        if command.is_empty() {
            return HealthStatus::Unknown;
        }

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
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

/// Spawn health checkers for all units that have health checks configured
pub fn spawn_health_checkers(
    units: &[Unit],
    event_tx: broadcast::Sender<EventEnvelope>,
    next_id: Arc<AtomicU64>,
) {
    for unit in units {
        if let Some(check) = &unit.health {
            let checker =
                HealthChecker::new(unit.id.clone(), check.clone(), unit.port, next_id.clone());
            let tx = event_tx.clone();
            tokio::spawn(async move {
                checker.run(tx).await;
            });
        }
    }
}
