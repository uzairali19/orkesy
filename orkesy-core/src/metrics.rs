//! Metrics time-series storage for live graphs
//!
//! Provides ring-buffer based storage for metrics data points,
//! designed for rendering real-time charts in the TUI.

use std::collections::{BTreeMap, VecDeque};

use crate::model::ServiceId;

/// A time-series of (timestamp, value) pairs with fixed capacity.
/// Oldest points are dropped when capacity is exceeded.
#[derive(Clone, Debug)]
pub struct Series {
    /// Maximum number of points to store
    pub cap: usize,
    /// Ring buffer of (t_seconds, value) pairs
    pub points: VecDeque<(f64, f64)>,
}

impl Series {
    /// Create a new series with given capacity
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            points: VecDeque::with_capacity(cap),
        }
    }

    /// Push a new data point, dropping oldest if at capacity
    pub fn push(&mut self, t: f64, v: f64) {
        if self.points.len() >= self.cap {
            self.points.pop_front();
        }
        self.points.push_back((t, v));
    }

    /// Get points as a Vec for Chart rendering
    pub fn as_vec(&self) -> Vec<(f64, f64)> {
        self.points.iter().copied().collect()
    }

    /// Get the most recent value, if any
    pub fn latest(&self) -> Option<f64> {
        self.points.back().map(|(_, v)| *v)
    }

    /// Get min and max timestamps in the series
    pub fn time_bounds(&self) -> Option<(f64, f64)> {
        if self.points.is_empty() {
            return None;
        }
        let min_t = self.points.front().map(|(t, _)| *t).unwrap_or(0.0);
        let max_t = self.points.back().map(|(t, _)| *t).unwrap_or(0.0);
        Some((min_t, max_t))
    }

    /// Get min and max values in the series
    pub fn value_bounds(&self) -> Option<(f64, f64)> {
        if self.points.is_empty() {
            return None;
        }
        let min_v = self
            .points
            .iter()
            .map(|(_, v)| *v)
            .fold(f64::INFINITY, f64::min);
        let max_v = self
            .points
            .iter()
            .map(|(_, v)| *v)
            .fold(f64::NEG_INFINITY, f64::max);
        Some((min_v, max_v))
    }

    /// Clear all points
    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Check if series is empty
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Get number of points
    pub fn len(&self) -> usize {
        self.points.len()
    }
}

impl Default for Series {
    fn default() -> Self {
        // Default: 120 points = 60 seconds at 500ms sampling
        Self::new(120)
    }
}

/// Aggregated metrics state for the entire runtime.
/// Stores time-series data for system-level and per-service metrics.
#[derive(Clone, Debug)]
pub struct MetricsState {
    /// System-wide CPU percentage (0-100)
    pub system_cpu: Series,
    /// System-wide memory usage in MB
    pub system_mem: Series,
    /// System-wide network throughput in KB/s
    pub system_net: Series,

    /// Per-service CPU percentage
    pub svc_cpu: BTreeMap<ServiceId, Series>,
    /// Per-service memory in MB
    pub svc_mem: BTreeMap<ServiceId, Series>,
    /// Per-service network KB/s
    pub svc_net: BTreeMap<ServiceId, Series>,

    /// Per-service log rate (logs/second)
    pub logs_rate: BTreeMap<ServiceId, Series>,

    /// Log counters for computing log rate (used by sampler)
    pub log_counts: BTreeMap<ServiceId, u64>,
    /// Previous log counts for delta calculation
    pub prev_log_counts: BTreeMap<ServiceId, u64>,
}

impl MetricsState {
    /// Create a new MetricsState with default capacity (120 points = 60s window)
    pub fn new() -> Self {
        Self::with_capacity(120)
    }

    /// Create with custom capacity
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            system_cpu: Series::new(cap),
            system_mem: Series::new(cap),
            system_net: Series::new(cap),
            svc_cpu: BTreeMap::new(),
            svc_mem: BTreeMap::new(),
            svc_net: BTreeMap::new(),
            logs_rate: BTreeMap::new(),
            log_counts: BTreeMap::new(),
            prev_log_counts: BTreeMap::new(),
        }
    }

    /// Push system-level metrics
    pub fn push_system(&mut self, t: f64, cpu_pct: f64, mem_mb: f64, net_kbps: f64) {
        self.system_cpu.push(t, cpu_pct);
        self.system_mem.push(t, mem_mb);
        self.system_net.push(t, net_kbps);
    }

    /// Push per-service metrics
    pub fn push_service(
        &mut self,
        t: f64,
        id: &ServiceId,
        cpu_pct: Option<f64>,
        mem_mb: Option<f64>,
        net_kbps: Option<f64>,
    ) {
        if let Some(cpu) = cpu_pct {
            self.svc_cpu
                .entry(id.clone())
                .or_insert_with(Series::default)
                .push(t, cpu);
        }
        if let Some(mem) = mem_mb {
            self.svc_mem
                .entry(id.clone())
                .or_insert_with(Series::default)
                .push(t, mem);
        }
        if let Some(net) = net_kbps {
            self.svc_net
                .entry(id.clone())
                .or_insert_with(Series::default)
                .push(t, net);
        }
    }

    /// Push log rate for a service
    pub fn push_log_rate(&mut self, t: f64, id: &ServiceId, logs_per_sec: f64) {
        self.logs_rate
            .entry(id.clone())
            .or_insert_with(Series::default)
            .push(t, logs_per_sec);
    }

    /// Increment log count for a service (called when log line received)
    pub fn increment_log_count(&mut self, id: &ServiceId) {
        *self.log_counts.entry(id.clone()).or_insert(0) += 1;
    }

    /// Compute log rates from count deltas (called by sampler)
    /// Returns map of service_id -> logs/second
    pub fn compute_log_rates(&mut self, interval_secs: f64) -> BTreeMap<ServiceId, f64> {
        let mut rates = BTreeMap::new();

        for (id, &count) in &self.log_counts {
            let prev = self.prev_log_counts.get(id).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            let rate = delta as f64 / interval_secs;
            rates.insert(id.clone(), rate);
        }

        // Update previous counts
        self.prev_log_counts = self.log_counts.clone();

        rates
    }

    /// Clear metrics for a service (when it stops)
    pub fn clear_service(&mut self, id: &ServiceId) {
        self.svc_cpu.remove(id);
        self.svc_mem.remove(id);
        self.svc_net.remove(id);
        // Keep logs_rate for history, but clear counts
        self.log_counts.remove(id);
        self.prev_log_counts.remove(id);
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_series_push_and_capacity() {
        let mut s = Series::new(3);
        s.push(1.0, 10.0);
        s.push(2.0, 20.0);
        s.push(3.0, 30.0);
        assert_eq!(s.len(), 3);

        s.push(4.0, 40.0);
        assert_eq!(s.len(), 3);
        assert_eq!(s.as_vec(), vec![(2.0, 20.0), (3.0, 30.0), (4.0, 40.0)]);
    }

    #[test]
    fn test_series_bounds() {
        let mut s = Series::new(10);
        s.push(1.0, 5.0);
        s.push(2.0, 15.0);
        s.push(3.0, 10.0);

        assert_eq!(s.time_bounds(), Some((1.0, 3.0)));
        assert_eq!(s.value_bounds(), Some((5.0, 15.0)));
    }

    #[test]
    fn test_series_empty() {
        let s = Series::new(10);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert_eq!(s.latest(), None);
        assert_eq!(s.time_bounds(), None);
        assert_eq!(s.value_bounds(), None);
    }

    #[test]
    fn test_series_clear() {
        let mut s = Series::new(10);
        s.push(1.0, 10.0);
        s.push(2.0, 20.0);
        assert_eq!(s.len(), 2);

        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn test_series_default() {
        let s = Series::default();
        assert_eq!(s.cap, 120);
        assert!(s.is_empty());
    }

    #[test]
    fn test_metrics_state_push_system() {
        let mut state = MetricsState::new();
        state.push_system(1.0, 50.0, 1024.0, 100.0);

        assert_eq!(state.system_cpu.latest(), Some(50.0));
        assert_eq!(state.system_mem.latest(), Some(1024.0));
        assert_eq!(state.system_net.latest(), Some(100.0));
    }

    #[test]
    fn test_metrics_state_push_service() {
        let mut state = MetricsState::new();
        state.push_service(1.0, &"api".to_string(), Some(25.0), Some(512.0), None);

        assert_eq!(state.svc_cpu.get("api").unwrap().latest(), Some(25.0));
        assert_eq!(state.svc_mem.get("api").unwrap().latest(), Some(512.0));
        assert!(state.svc_net.get("api").is_none());
    }

    #[test]
    fn test_metrics_state_log_rate_calculation() {
        let mut state = MetricsState::new();

        state.increment_log_count(&"api".to_string());
        state.increment_log_count(&"api".to_string());
        state.increment_log_count(&"api".to_string());

        let rates = state.compute_log_rates(1.0);
        assert_eq!(rates.get("api"), Some(&3.0));

        state.increment_log_count(&"api".to_string());
        state.increment_log_count(&"api".to_string());

        let rates = state.compute_log_rates(0.5);
        assert_eq!(rates.get("api"), Some(&4.0)); // 2 logs / 0.5s = 4/s
    }

    #[test]
    fn test_metrics_state_clear_service() {
        let mut state = MetricsState::new();

        state.push_service(1.0, &"api".to_string(), Some(25.0), Some(512.0), Some(50.0));
        state.increment_log_count(&"api".to_string());

        assert!(state.svc_cpu.contains_key("api"));
        assert!(state.log_counts.contains_key("api"));

        state.clear_service(&"api".to_string());

        assert!(!state.svc_cpu.contains_key("api"));
        assert!(!state.svc_mem.contains_key("api"));
        assert!(!state.svc_net.contains_key("api"));
        assert!(!state.log_counts.contains_key("api"));
    }

    #[test]
    fn test_metrics_state_with_capacity() {
        let state = MetricsState::with_capacity(60);
        assert_eq!(state.system_cpu.cap, 60);
        assert_eq!(state.system_mem.cap, 60);
        assert_eq!(state.system_net.cap, 60);
    }
}
