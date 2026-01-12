use std::time::SystemTime;

use crate::command::{CommandId, CommandRun, ProjectIndex, RunId, RunStatus};
use crate::model::{HealthStatus, RuntimeGraph, ServiceId, ServiceStatus};
use crate::state::{LogLine, LogStream, RuntimeState};
use crate::unit::UnitMetrics;

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
        stream: LogStream,
        text: String,
    },
    ClearLogs {
        id: ServiceId,
    },
    MetricsUpdated {
        id: ServiceId,
        metrics: UnitMetrics,
    },

    // Commands + Runs feature
    /// Project was indexed (tools and commands detected)
    ProjectIndexed {
        project: ProjectIndex,
    },
    /// A command started running
    CommandStarted {
        run_id: RunId,
        command_id: CommandId,
        command: String,
        display_name: String,
        pid: Option<u32>,
    },
    /// Output from a running command
    CommandOutput {
        run_id: RunId,
        stream: LogStream,
        text: String,
    },
    /// A command finished (exited normally)
    CommandFinished {
        run_id: RunId,
        exit_code: Option<i32>,
    },
    /// A command was killed
    CommandKilled {
        run_id: RunId,
    },
    /// Clear logs for a command run
    ClearRunLogs {
        run_id: RunId,
    },

    // Time-series metrics for graphs
    /// System-wide metrics sample
    SystemMetricsSample {
        t: f64,
        cpu_pct: f64,
        mem_mb: f64,
        net_kbps: f64,
    },
    /// Per-service metrics sample
    ServiceMetricsSample {
        t: f64,
        id: ServiceId,
        cpu_pct: Option<f64>,
        mem_mb: Option<f64>,
        net_kbps: Option<f64>,
    },
    /// Log rate sample for a service
    LogRateSample {
        t: f64,
        id: ServiceId,
        per_sec: f64,
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
            // Clear metrics when service is no longer running
            match status {
                ServiceStatus::Stopped
                | ServiceStatus::Exited { .. }
                | ServiceStatus::Errored { .. } => {
                    state.metrics.remove(id);
                    state.metrics_series.clear_service(id);
                }
                _ => {}
            }
        }
        RuntimeEvent::HealthChanged { id, health } => {
            if let Some(node) = state.graph.nodes.get_mut(id) {
                node.observed.health = health.clone();
            }
        }
        RuntimeEvent::LogLine { id, stream, text } => {
            state.logs.push(
                id,
                LogLine {
                    at: env.at,
                    service_id: id.clone(),
                    stream: *stream,
                    text: text.clone(),
                },
            );
            // Increment log count for rate calculation
            state.metrics_series.increment_log_count(id);
        }
        RuntimeEvent::ClearLogs { id } => state.logs.clear(id),
        RuntimeEvent::MetricsUpdated { id, metrics } => {
            state.metrics.insert(id.clone(), metrics.clone());
        }

        // Commands + Runs feature
        RuntimeEvent::ProjectIndexed { project } => {
            state.project = Some(project.clone());
        }
        RuntimeEvent::CommandStarted {
            run_id,
            command_id,
            command,
            display_name,
            pid,
        } => {
            let run = CommandRun {
                id: run_id.clone(),
                command_id: command_id.clone(),
                command: command.clone(),
                display_name: display_name.clone(),
                status: RunStatus::Running,
                started_at: env.at,
                finished_at: None,
                exit_code: None,
                pid: *pid,
            };
            state.add_run(run);
        }
        RuntimeEvent::CommandOutput {
            run_id,
            stream,
            text,
        } => {
            state.logs.push_run(
                run_id,
                LogLine {
                    at: env.at,
                    service_id: run_id.clone(), // Reuse field for run_id
                    stream: *stream,
                    text: text.clone(),
                },
            );
        }
        RuntimeEvent::CommandFinished { run_id, exit_code } => {
            if let Some(run) = state.runs.get_mut(run_id) {
                run.status = RunStatus::Exited { code: *exit_code };
                run.finished_at = Some(env.at);
                run.exit_code = *exit_code;
            }
        }
        RuntimeEvent::CommandKilled { run_id } => {
            if let Some(run) = state.runs.get_mut(run_id) {
                run.status = RunStatus::Killed;
                run.finished_at = Some(env.at);
            }
        }
        RuntimeEvent::ClearRunLogs { run_id } => {
            state.logs.clear_run(run_id);
        }

        // Time-series metrics for graphs
        RuntimeEvent::SystemMetricsSample {
            t,
            cpu_pct,
            mem_mb,
            net_kbps,
        } => {
            state
                .metrics_series
                .push_system(*t, *cpu_pct, *mem_mb, *net_kbps);
        }
        RuntimeEvent::ServiceMetricsSample {
            t,
            id,
            cpu_pct,
            mem_mb,
            net_kbps,
        } => {
            state
                .metrics_series
                .push_service(*t, id, *cpu_pct, *mem_mb, *net_kbps);
        }
        RuntimeEvent::LogRateSample { t, id, per_sec } => {
            state.metrics_series.push_log_rate(*t, id, *per_sec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DesiredState, ObservedState, RuntimeGraph, ServiceKind, ServiceNode};
    use std::collections::{BTreeMap, BTreeSet};

    fn make_envelope(id: u64, event: RuntimeEvent) -> EventEnvelope {
        EventEnvelope {
            id,
            at: SystemTime::now(),
            event,
        }
    }

    fn make_test_graph() -> RuntimeGraph {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "api".to_string(),
            ServiceNode {
                id: "api".to_string(),
                display_name: "API Server".to_string(),
                kind: ServiceKind::HttpApi,
                desired: DesiredState::Running,
                observed: ObservedState {
                    instance_id: None,
                    status: ServiceStatus::Stopped,
                    health: HealthStatus::Unknown,
                },
                port: Some(8080),
                description: Some("Main API".to_string()),
            },
        );
        RuntimeGraph {
            nodes,
            edges: BTreeSet::new(),
        }
    }

    #[test]
    fn test_topology_loaded() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(RuntimeGraph {
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
        });

        let env = make_envelope(
            1,
            RuntimeEvent::TopologyLoaded {
                graph: graph.clone(),
            },
        );
        reduce(&mut state, &env);

        assert_eq!(state.graph.nodes.len(), 1);
        assert!(state.graph.nodes.contains_key("api"));
        assert_eq!(state.last_event_id, 1);
    }

    #[test]
    fn test_status_changed() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        let env = make_envelope(
            1,
            RuntimeEvent::StatusChanged {
                id: "api".to_string(),
                status: ServiceStatus::Running,
            },
        );
        reduce(&mut state, &env);

        let node = state.graph.nodes.get("api").unwrap();
        assert!(matches!(node.observed.status, ServiceStatus::Running));
    }

    #[test]
    fn test_status_stopped_clears_metrics() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        state.metrics.insert(
            "api".to_string(),
            UnitMetrics {
                cpu_percent: 50.0,
                memory_bytes: 1024,
                uptime_secs: 100,
                pid: Some(1234),
            },
        );

        let env = make_envelope(
            1,
            RuntimeEvent::StatusChanged {
                id: "api".to_string(),
                status: ServiceStatus::Stopped,
            },
        );
        reduce(&mut state, &env);

        assert!(!state.metrics.contains_key("api"));
    }

    #[test]
    fn test_health_changed() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        let env = make_envelope(
            1,
            RuntimeEvent::HealthChanged {
                id: "api".to_string(),
                health: HealthStatus::Healthy,
            },
        );
        reduce(&mut state, &env);

        let node = state.graph.nodes.get("api").unwrap();
        assert!(matches!(node.observed.health, HealthStatus::Healthy));
    }

    #[test]
    fn test_log_line() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        let env = make_envelope(
            1,
            RuntimeEvent::LogLine {
                id: "api".to_string(),
                stream: LogStream::Stdout,
                text: "Server started".to_string(),
            },
        );
        reduce(&mut state, &env);

        assert_eq!(state.logs.per_service.get("api").unwrap().len(), 1);
        assert_eq!(state.logs.merged.len(), 1);
    }

    #[test]
    fn test_clear_logs() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        let env = make_envelope(
            1,
            RuntimeEvent::LogLine {
                id: "api".to_string(),
                stream: LogStream::Stdout,
                text: "Log line".to_string(),
            },
        );
        reduce(&mut state, &env);

        let env = make_envelope(
            2,
            RuntimeEvent::ClearLogs {
                id: "api".to_string(),
            },
        );
        reduce(&mut state, &env);

        assert!(!state.logs.per_service.contains_key("api"));
    }

    #[test]
    fn test_metrics_updated() {
        let graph = make_test_graph();
        let mut state = RuntimeState::new(graph);

        let metrics = UnitMetrics {
            cpu_percent: 25.5,
            memory_bytes: 1024 * 1024,
            uptime_secs: 3600,
            pid: Some(5678),
        };

        let env = make_envelope(
            1,
            RuntimeEvent::MetricsUpdated {
                id: "api".to_string(),
                metrics: metrics.clone(),
            },
        );
        reduce(&mut state, &env);

        let stored = state.metrics.get("api").unwrap();
        assert_eq!(stored.cpu_percent, 25.5);
        assert_eq!(stored.pid, Some(5678));
    }

    #[test]
    fn test_system_metrics_sample() {
        let mut state = RuntimeState::new(make_test_graph());

        let env = make_envelope(
            1,
            RuntimeEvent::SystemMetricsSample {
                t: 1.0,
                cpu_pct: 45.0,
                mem_mb: 1024.0,
                net_kbps: 100.0,
            },
        );
        reduce(&mut state, &env);

        assert_eq!(state.metrics_series.system_cpu.len(), 1);
        assert_eq!(state.metrics_series.system_cpu.latest(), Some(45.0));
    }

    #[test]
    fn test_log_rate_sample() {
        let mut state = RuntimeState::new(make_test_graph());

        let env = make_envelope(
            1,
            RuntimeEvent::LogRateSample {
                t: 1.0,
                id: "api".to_string(),
                per_sec: 10.5,
            },
        );
        reduce(&mut state, &env);

        let rate = state.metrics_series.logs_rate.get("api").unwrap();
        assert_eq!(rate.latest(), Some(10.5));
    }

    #[test]
    fn test_command_lifecycle() {
        let mut state = RuntimeState::new(make_test_graph());

        // Start command
        let env = make_envelope(
            1,
            RuntimeEvent::CommandStarted {
                run_id: "run-1".to_string(),
                command_id: "test".to_string(),
                command: "npm test".to_string(),
                display_name: "Test".to_string(),
                pid: Some(1234),
            },
        );
        reduce(&mut state, &env);

        assert!(state.runs.contains_key("run-1"));
        assert!(matches!(
            state.runs.get("run-1").unwrap().status,
            RunStatus::Running
        ));

        // Finish command
        let env = make_envelope(
            2,
            RuntimeEvent::CommandFinished {
                run_id: "run-1".to_string(),
                exit_code: Some(0),
            },
        );
        reduce(&mut state, &env);

        let run = state.runs.get("run-1").unwrap();
        assert!(matches!(run.status, RunStatus::Exited { code: Some(0) }));
        assert!(run.finished_at.is_some());
    }

    #[test]
    fn test_command_killed() {
        let mut state = RuntimeState::new(make_test_graph());

        let env = make_envelope(
            1,
            RuntimeEvent::CommandStarted {
                run_id: "run-1".to_string(),
                command_id: "test".to_string(),
                command: "npm test".to_string(),
                display_name: "Test".to_string(),
                pid: Some(1234),
            },
        );
        reduce(&mut state, &env);

        let env = make_envelope(
            2,
            RuntimeEvent::CommandKilled {
                run_id: "run-1".to_string(),
            },
        );
        reduce(&mut state, &env);

        let run = state.runs.get("run-1").unwrap();
        assert!(matches!(run.status, RunStatus::Killed));
    }
}
