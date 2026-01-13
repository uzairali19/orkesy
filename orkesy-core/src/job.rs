use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::command::{CommandId, UnitId};
use crate::state::LogStream;

pub type JobId = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    Queued,
    Running,
    Success,
    Failed { code: Option<i32> },
    Cancelled,
}

impl JobStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            JobStatus::Queued => "◯",
            JobStatus::Running => "●",
            JobStatus::Success => "✓",
            JobStatus::Failed { .. } => "✗",
            JobStatus::Cancelled => "⊘",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Success => "success",
            JobStatus::Failed { .. } => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Running)
    }

    pub fn is_success(&self) -> bool {
        matches!(self, JobStatus::Success)
    }

    pub fn is_finished(&self) -> bool {
        !self.is_active()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub command_id: Option<CommandId>,
    pub unit_id: Option<UnitId>,
    pub display_name: String,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub created_at: SystemTime,
}

impl JobSpec {
    pub fn from_shell_command(command: &str, display_name: &str) -> Self {
        Self {
            command_id: None,
            unit_id: None,
            display_name: display_name.to_string(),
            argv: vec!["sh".to_string(), "-c".to_string(), command.to_string()],
            cwd: None,
            env: BTreeMap::new(),
            created_at: SystemTime::now(),
        }
    }

    pub fn with_unit(mut self, unit_id: &str) -> Self {
        self.unit_id = Some(unit_id.to_string());
        self
    }

    pub fn with_command(mut self, command_id: &str) -> Self {
        self.command_id = Some(command_id.to_string());
        self
    }

    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub spec: JobSpec,
    pub status: JobStatus,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
}

impl Job {
    pub fn new(id: JobId, spec: JobSpec) -> Self {
        Self {
            id,
            spec,
            status: JobStatus::Queued,
            started_at: None,
            finished_at: None,
            exit_code: None,
            pid: None,
        }
    }

    pub fn mark_started(&mut self, pid: Option<u32>) {
        self.status = JobStatus::Running;
        self.started_at = Some(SystemTime::now());
        self.pid = pid;
    }

    pub fn mark_finished(&mut self, exit_code: Option<i32>) {
        self.finished_at = Some(SystemTime::now());
        self.exit_code = exit_code;
        self.status = match exit_code {
            Some(0) => JobStatus::Success,
            _ => JobStatus::Failed { code: exit_code },
        };
    }

    pub fn mark_cancelled(&mut self) {
        self.finished_at = Some(SystemTime::now());
        self.status = JobStatus::Cancelled;
    }

    pub fn duration(&self) -> Option<std::time::Duration> {
        self.started_at.map(|start| {
            let end = self.finished_at.unwrap_or_else(SystemTime::now);
            end.duration_since(start).unwrap_or_default()
        })
    }

    pub fn duration_str(&self) -> String {
        match self.duration() {
            Some(d) => {
                let secs = d.as_secs();
                if secs < 60 {
                    format!("{}s", secs)
                } else if secs < 3600 {
                    format!("{}m {}s", secs / 60, secs % 60)
                } else {
                    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                }
            }
            None => "-".to_string(),
        }
    }

    pub fn display_name(&self) -> &str {
        &self.spec.display_name
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobEvent {
    JobQueued {
        job_id: JobId,
        spec: JobSpec,
    },
    JobStarted {
        job_id: JobId,
        pid: Option<u32>,
    },
    JobOutput {
        job_id: JobId,
        stream: LogStream,
        line: String,
    },
    JobFinished {
        job_id: JobId,
        status: JobStatus,
        exit_code: Option<i32>,
    },
}

const MAX_COMPLETED_JOBS: usize = 50;

#[derive(Clone, Debug, Default)]
pub struct JobStore {
    jobs: BTreeMap<JobId, Job>,
    order: Vec<JobId>,
}

impl JobStore {
    pub fn new() -> Self {
        Self {
            jobs: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    pub fn add(&mut self, job: Job) {
        let id = job.id.clone();
        self.jobs.insert(id.clone(), job);
        self.order.insert(0, id);
        self.prune_completed();
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.get_mut(id)
    }

    pub fn active(&self) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.status.is_active())
            .collect()
    }

    pub fn completed(&self) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.status.is_finished())
            .collect()
    }

    pub fn for_unit(&self, unit_id: &str) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.spec.unit_id.as_deref() == Some(unit_id))
            .collect()
    }

    pub fn recent(&self, limit: usize) -> Vec<&Job> {
        let mut result: Vec<&Job> = Vec::with_capacity(limit);

        for id in &self.order {
            if let Some(job) = self.jobs.get(id)
                && job.status.is_active()
            {
                result.push(job);
                if result.len() >= limit {
                    return result;
                }
            }
        }

        for id in &self.order {
            if let Some(job) = self.jobs.get(id)
                && job.status.is_finished()
            {
                result.push(job);
                if result.len() >= limit {
                    return result;
                }
            }
        }

        result
    }

    pub fn active_count(&self) -> usize {
        self.jobs.values().filter(|j| j.status.is_active()).count()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn apply_event(&mut self, event: &JobEvent) {
        match event {
            JobEvent::JobQueued { job_id, spec } => {
                let job = Job::new(job_id.clone(), spec.clone());
                self.add(job);
            }
            JobEvent::JobStarted { job_id, pid } => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.mark_started(*pid);
                }
            }
            JobEvent::JobOutput { .. } => {}
            JobEvent::JobFinished {
                job_id,
                status,
                exit_code,
            } => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.status = status.clone();
                    job.exit_code = *exit_code;
                    job.finished_at = Some(SystemTime::now());
                }
            }
        }
    }

    fn prune_completed(&mut self) {
        let completed_count = self
            .jobs
            .values()
            .filter(|j| j.status.is_finished())
            .count();

        if completed_count <= MAX_COMPLETED_JOBS {
            return;
        }

        let to_remove: Vec<JobId> = self
            .order
            .iter()
            .rev()
            .filter(|id| {
                self.jobs
                    .get(*id)
                    .map(|j| j.status.is_finished())
                    .unwrap_or(false)
            })
            .take(completed_count - MAX_COMPLETED_JOBS)
            .cloned()
            .collect();

        for id in to_remove {
            self.jobs.remove(&id);
            self.order.retain(|x| x != &id);
        }
    }
}

pub fn job_log_prefix(job: &Job) -> String {
    if let Some(unit_id) = &job.spec.unit_id {
        format!("{} > {}", unit_id, job.display_name())
    } else {
        format!("job:{}", &job.id[..8.min(job.id.len())])
    }
}

pub fn format_job_log_line(job: &Job, line: &str) -> String {
    format!("{} | {}", job_log_prefix(job), line)
}
