use std::collections::{ BTreeMap, VecDeque };
use std::time::SystemTime;

use crate::model::{ RuntimeGraph, ServiceId };

#[derive(Clone, Debug)]
pub struct LogLine {
    pub at: SystemTime,
    pub text: String,
}

#[derive(Debug)]
pub struct LogStore {
    pub cap: usize,
    pub per_service: BTreeMap<ServiceId, VecDeque<LogLine>>,
}

impl LogStore {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            per_service: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, id: &ServiceId, line: LogLine) {
        let q = self.per_service.entry(id.clone()).or_default();
        q.push_back(line);
        while q.len() > self.cap {
            q.pop_front();
        }
    }

    pub fn clear(&mut self, id: &ServiceId) {
        self.per_service.remove(id);
    }
}

#[derive(Debug)]
pub struct RuntimeState {
    pub graph: RuntimeGraph,
    pub logs: LogStore,
    pub last_event_id: u64,
}

impl RuntimeState {
    pub fn new(graph: RuntimeGraph) -> Self {
        Self {
            graph,
            logs: LogStore::new(10_000),
            last_event_id: 0,
        }
    }
}
