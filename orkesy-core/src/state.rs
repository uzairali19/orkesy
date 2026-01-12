use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use crate::command::{CommandRun, ProjectIndex, RunId};
use crate::metrics::MetricsState;
use crate::model::{RuntimeGraph, ServiceId};
use crate::unit::UnitMetrics;

/// Which output stream a log line came from
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub at: SystemTime,
    pub service_id: String,
    pub stream: LogStream,
    pub text: String,
}

#[derive(Debug)]
pub struct LogStore {
    pub cap: usize,
    pub per_service: BTreeMap<ServiceId, VecDeque<LogLine>>,
    pub merged: VecDeque<LogLine>,
    /// Logs for command runs (Commands + Runs feature)
    pub per_run: BTreeMap<RunId, VecDeque<LogLine>>,
}

impl LogStore {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            per_service: BTreeMap::new(),
            merged: VecDeque::new(),
            per_run: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, id: &ServiceId, line: LogLine) {
        // Push to per-service buffer
        let q = self.per_service.entry(id.clone()).or_default();
        q.push_back(line.clone());
        while q.len() > self.cap {
            q.pop_front();
        }

        // Push to merged buffer
        self.merged.push_back(line);
        while self.merged.len() > self.cap {
            self.merged.pop_front();
        }
    }

    pub fn clear(&mut self, id: &ServiceId) {
        self.per_service.remove(id);
        // Note: merged logs are not cleared per-service (would be expensive)
    }

    /// Push a log line for a command run
    pub fn push_run(&mut self, run_id: &RunId, line: LogLine) {
        let q = self.per_run.entry(run_id.clone()).or_default();
        q.push_back(line);
        while q.len() > self.cap {
            q.pop_front();
        }
    }

    /// Clear logs for a command run
    pub fn clear_run(&mut self, run_id: &RunId) {
        self.per_run.remove(run_id);
    }
}

/// Maximum number of command runs to keep in history
const MAX_RUNS: usize = 200;

#[derive(Debug)]
pub struct RuntimeState {
    pub graph: RuntimeGraph,
    pub logs: LogStore,
    pub metrics: BTreeMap<ServiceId, UnitMetrics>,
    pub last_event_id: u64,

    // Commands + Runs feature
    /// Indexed project information (detected tools and commands)
    pub project: Option<ProjectIndex>,
    /// Command runs indexed by run ID
    pub runs: BTreeMap<RunId, CommandRun>,
    /// Run IDs in order (most recent first)
    pub run_order: Vec<RunId>,

    // Time-series metrics for graphs
    /// Ring-buffer metrics storage for live charts
    pub metrics_series: MetricsState,
}

impl RuntimeState {
    pub fn new(graph: RuntimeGraph) -> Self {
        Self {
            graph,
            logs: LogStore::new(10_000),
            metrics: BTreeMap::new(),
            last_event_id: 0,
            project: None,
            runs: BTreeMap::new(),
            run_order: Vec::new(),
            metrics_series: MetricsState::new(),
        }
    }

    /// Add a command run to history (maintains MAX_RUNS limit)
    pub fn add_run(&mut self, run: CommandRun) {
        let run_id = run.id.clone();
        self.runs.insert(run_id.clone(), run);
        self.run_order.insert(0, run_id);

        // Trim old runs if over limit
        while self.run_order.len() > MAX_RUNS {
            if let Some(old_id) = self.run_order.pop() {
                self.runs.remove(&old_id);
                self.logs.clear_run(&old_id);
            }
        }
    }

    /// Get runs in order (most recent first)
    pub fn runs_ordered(&self) -> Vec<&CommandRun> {
        self.run_order
            .iter()
            .filter_map(|id| self.runs.get(id))
            .collect()
    }
}
