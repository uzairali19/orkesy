use std::time::SystemTime;

use crate::model::{ HealthStatus, RuntimeGraph, ServiceId, ServiceStatus };
use crate::state::{ LogLine, RuntimeState };

#[derive(Clone, Debug)]
pub enum RuntimeEvent {
    TopologyLoaded {
        graph: RuntimeGraph,
    },
    StatusChanged {
        id: ServiceId,
        status: ServiceStatus,
    },
    HealthChanged {
        id: ServiceId,
        health: HealthStatus,
    },
    LogLine {
        id: ServiceId,
        text: String,
    },
    ClearLogs {
        id: ServiceId,
    },
}

#[derive(Clone, Debug)]
pub struct EventEnvelope {
    pub id: u64,
    pub at: SystemTime,
    pub event: RuntimeEvent,
}

pub fn reduce(state: &mut RuntimeState, env: &EventEnvelope) {
    state.last_event_id = env.id;

    match &env.event {
        RuntimeEvent::TopologyLoaded { graph } => {
            state.graph = graph.clone();
        }
        RuntimeEvent::StatusChanged { id, status } => {
            if let Some(node) = state.graph.nodes.get_mut(id) {
                node.observed.status = status.clone();
            }
        }
        RuntimeEvent::HealthChanged { id, health } => {
            if let Some(node) = state.graph.nodes.get_mut(id) {
                node.observed.health = health.clone();
            }
        }
        RuntimeEvent::LogLine { id, text } => {
            state.logs.push(id, LogLine {
                at: env.at,
                text: text.clone(),
            });
        }
        RuntimeEvent::ClearLogs { id } => state.logs.clear(id),
    }
}
