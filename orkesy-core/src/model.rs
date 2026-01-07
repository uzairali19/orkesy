use std::collections::{ BTreeMap, BTreeSet };

pub type ServiceId = String;
pub type InstanceId = String;

#[derive(Clone, Debug)]
pub enum ServiceStatus {
    Unknown,
    Starting,
    Running,
    Stopped,
    Exited {
        code: Option<i32>,
    },
    Restarting,
    Errored {
        message: String,
    },
}

#[derive(Clone, Debug)]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Degraded {
        reason: String,
    },
    Unhealthy {
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub enum ServiceKind {
    HttpApi,
    Worker,
    Database,
    Cache,
    Queue,
    Frontend,
    Generic,
}

#[derive(Clone, Debug)]
pub enum DesiredState {
    Running,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct ObservedState {
    pub instance_id: Option<InstanceId>,
    pub status: ServiceStatus,
    pub health: HealthStatus,
}

#[derive(Clone, Debug)]
pub struct ServiceNode {
    pub id: ServiceId,
    pub display_name: String,
    pub kind: ServiceKind,
    pub desired: DesiredState,
    pub observed: ObservedState,
    /// Port the service listens on (informational)
    pub port: Option<u16>,
    /// Description for display in UI
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    DependsOn,
    TalksTo,
    Produces,
    Consumes,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub from: ServiceId,
    pub to: ServiceId,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug)]
pub struct RuntimeGraph {
    pub nodes: BTreeMap<ServiceId, ServiceNode>,
    pub edges: BTreeSet<Edge>,
}
