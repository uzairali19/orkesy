use std::sync::Arc;
use std::time::{Duration, Instant};

use sysinfo::{Networks, System};
use tokio::sync::{RwLock, broadcast};

use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::RuntimeState;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

pub struct MetricsSampler {
    system: System,
    networks: Networks,
    start_instant: Instant,
    event_id: u64,
    prev_net_rx: u64,
    prev_net_tx: u64,
}

impl MetricsSampler {
    pub fn new() -> Self {
        Self {
            system: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            start_instant: Instant::now(),
            event_id: 1_000_000, // Start high to avoid collision with other event sources
            prev_net_rx: 0,
            prev_net_tx: 0,
        }
    }

    fn timestamp(&self) -> f64 {
        self.start_instant.elapsed().as_secs_f64()
    }

    fn next_event_id(&mut self) -> u64 {
        let id = self.event_id;
        self.event_id += 1;
        id
    }

    fn sample_system(&mut self) -> RuntimeEvent {
        // Refresh system info
        self.system.refresh_cpu_all();
        self.system.refresh_memory();
        self.networks.refresh();

        // CPU percentage (average across all cores)
        let cpu_pct = self.system.global_cpu_usage() as f64;

        // Memory in MB
        let used_mem = self.system.used_memory();
        let mem_mb = used_mem as f64 / (1024.0 * 1024.0);

        // Network throughput in KB/s
        let (total_rx, total_tx) = self
            .networks
            .iter()
            .fold((0u64, 0u64), |(rx, tx), (_, data)| {
                (rx + data.total_received(), tx + data.total_transmitted())
            });

        // Calculate delta and convert to KB/s
        let rx_delta = total_rx.saturating_sub(self.prev_net_rx);
        let tx_delta = total_tx.saturating_sub(self.prev_net_tx);
        let net_kbps = (rx_delta + tx_delta) as f64 / 1024.0 / SAMPLE_INTERVAL.as_secs_f64();

        self.prev_net_rx = total_rx;
        self.prev_net_tx = total_tx;

        RuntimeEvent::SystemMetricsSample {
            t: self.timestamp(),
            cpu_pct,
            mem_mb,
            net_kbps,
        }
    }

    fn compute_log_rates(
        &self,
        state: &mut orkesy_core::metrics::MetricsState,
    ) -> Vec<RuntimeEvent> {
        let t = self.timestamp();
        let interval_secs = SAMPLE_INTERVAL.as_secs_f64();

        let rates = state.compute_log_rates(interval_secs);

        rates
            .into_iter()
            .map(|(id, per_sec)| RuntimeEvent::LogRateSample { t, id, per_sec })
            .collect()
    }

    pub async fn run(
        mut self,
        event_tx: broadcast::Sender<EventEnvelope>,
        state: Arc<RwLock<RuntimeState>>,
    ) {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);

        // Skip the first tick (happens immediately)
        interval.tick().await;

        loop {
            interval.tick().await;

            // Sample system metrics
            let system_event = self.sample_system();
            let _ = event_tx.send(EventEnvelope {
                id: self.next_event_id(),
                at: std::time::SystemTime::now(),
                event: system_event,
            });

            // Compute and emit log rates
            {
                let mut state_guard = state.write().await;
                let log_rate_events = self.compute_log_rates(&mut state_guard.metrics_series);
                drop(state_guard);

                for event in log_rate_events {
                    let _ = event_tx.send(EventEnvelope {
                        id: self.next_event_id(),
                        at: std::time::SystemTime::now(),
                        event,
                    });
                }
            }
        }
    }
}

impl Default for MetricsSampler {
    fn default() -> Self {
        Self::new()
    }
}

pub fn spawn_sampler(event_tx: broadcast::Sender<EventEnvelope>, state: Arc<RwLock<RuntimeState>>) {
    let sampler = MetricsSampler::new();
    tokio::spawn(async move {
        sampler.run(event_tx, state).await;
    });
}
