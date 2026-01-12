//! Job Execution Model
//!
//! Provides types for reliable command execution with output streaming,
//! status tracking, and lifecycle management.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::command::{CommandId, UnitId};
use crate::state::LogStream;

/// Unique identifier for a job (UUID string)
pub type JobId = String;

/// Status of a job execution
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    /// Job is queued but not yet started
    Queued,
    /// Job is currently running
    Running,
    /// Job completed successfully (exit code 0)
    Success,
    /// Job failed with optional exit code
    Failed { code: Option<i32> },
    /// Job was cancelled by user
    Cancelled,
}

impl JobStatus {
    /// Icon for the status
    pub fn icon(&self) -> &'static str {
        match self {
            JobStatus::Queued => "◯",
            JobStatus::Running => "●",
            JobStatus::Success => "✓",
            JobStatus::Failed { .. } => "✗",
            JobStatus::Cancelled => "⊘",
        }
    }

    /// Short label for display
    pub fn label(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Success => "success",
            JobStatus::Failed { .. } => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    /// Whether the job is still active (not finished)
    pub fn is_active(&self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Running)
    }

    /// Whether the job completed successfully
    pub fn is_success(&self) -> bool {
        matches!(self, JobStatus::Success)
    }

    /// Whether the job finished (regardless of success/failure)
    pub fn is_finished(&self) -> bool {
        !self.is_active()
    }
}

/// Specification for creating a job
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    /// Optional reference to the command that spawned this job
    pub command_id: Option<CommandId>,
    /// Optional unit this job is associated with
    pub unit_id: Option<UnitId>,
    /// Display name for the job (shown in UI)
    pub display_name: String,
    /// Arguments to execute (first element is program)
    pub argv: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables to set
    pub env: BTreeMap<String, String>,
    /// When this spec was created
    pub created_at: SystemTime,
}

impl JobSpec {
    /// Create a new job spec from a shell command string
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

    /// Set the associated unit
    pub fn with_unit(mut self, unit_id: &str) -> Self {
        self.unit_id = Some(unit_id.to_string());
        self
    }

    /// Set the associated command
    pub fn with_command(mut self, command_id: &str) -> Self {
        self.command_id = Some(command_id.to_string());
        self
    }

    /// Set the working directory
    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    /// Add environment variables
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

/// A job instance (running or completed)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    /// Unique job ID
    pub id: JobId,
    /// The specification used to create this job
    pub spec: JobSpec,
    /// Current status
    pub status: JobStatus,
    /// When the job started running (if started)
    pub started_at: Option<SystemTime>,
    /// When the job finished (if finished)
    pub finished_at: Option<SystemTime>,
    /// Exit code (if exited)
    pub exit_code: Option<i32>,
    /// Process ID (if running)
    pub pid: Option<u32>,
}

impl Job {
    /// Create a new queued job
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

    /// Mark the job as started
    pub fn mark_started(&mut self, pid: Option<u32>) {
        self.status = JobStatus::Running;
        self.started_at = Some(SystemTime::now());
        self.pid = pid;
    }

    /// Mark the job as finished
    pub fn mark_finished(&mut self, exit_code: Option<i32>) {
        self.finished_at = Some(SystemTime::now());
        self.exit_code = exit_code;
        self.status = match exit_code {
            Some(0) => JobStatus::Success,
            _ => JobStatus::Failed { code: exit_code },
        };
    }

    /// Mark the job as cancelled
    pub fn mark_cancelled(&mut self) {
        self.finished_at = Some(SystemTime::now());
        self.status = JobStatus::Cancelled;
    }

    /// Duration of the job (from start to finish, or start to now)
    pub fn duration(&self) -> Option<std::time::Duration> {
        self.started_at.map(|start| {
            let end = self.finished_at.unwrap_or_else(SystemTime::now);
            end.duration_since(start).unwrap_or_default()
        })
    }

    /// Format duration as human-readable string
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

    /// Get the display name, falling back to command_id or a truncated argv
    pub fn display_name(&self) -> &str {
        &self.spec.display_name
    }
}

// ============================================================================
// Job Events - for the reducer
// ============================================================================

/// Events related to job lifecycle
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobEvent {
    /// Job has been created and queued
    JobQueued { job_id: JobId, spec: JobSpec },
    /// Job has started executing
    JobStarted { job_id: JobId, pid: Option<u32> },
    /// Job produced output
    JobOutput {
        job_id: JobId,
        stream: LogStream,
        line: String,
    },
    /// Job has finished
    JobFinished {
        job_id: JobId,
        status: JobStatus,
        exit_code: Option<i32>,
    },
}

// ============================================================================
// JobStore - holds job state
// ============================================================================

/// Maximum number of completed jobs to retain
const MAX_COMPLETED_JOBS: usize = 50;

/// Store for job state
#[derive(Clone, Debug, Default)]
pub struct JobStore {
    /// All jobs indexed by ID
    jobs: BTreeMap<JobId, Job>,
    /// Order of job IDs (most recent first)
    order: Vec<JobId>,
}

impl JobStore {
    /// Create a new empty job store
    pub fn new() -> Self {
        Self {
            jobs: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    /// Add a new job
    pub fn add(&mut self, job: Job) {
        let id = job.id.clone();
        self.jobs.insert(id.clone(), job);
        self.order.insert(0, id);
        self.prune_completed();
    }

    /// Get a job by ID
    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.get(id)
    }

    /// Get a mutable job by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.get_mut(id)
    }

    /// Get all active (running or queued) jobs
    pub fn active(&self) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.status.is_active())
            .collect()
    }

    /// Get all completed jobs (most recent first)
    pub fn completed(&self) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.status.is_finished())
            .collect()
    }

    /// Get jobs for a specific unit
    pub fn for_unit(&self, unit_id: &str) -> Vec<&Job> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .filter(|j| j.spec.unit_id.as_deref() == Some(unit_id))
            .collect()
    }

    /// Get recent jobs (active first, then completed, up to limit)
    pub fn recent(&self, limit: usize) -> Vec<&Job> {
        let mut result: Vec<&Job> = Vec::with_capacity(limit);

        // First add active jobs
        for id in &self.order {
            if let Some(job) = self.jobs.get(id) {
                if job.status.is_active() {
                    result.push(job);
                    if result.len() >= limit {
                        return result;
                    }
                }
            }
        }

        // Then add completed jobs
        for id in &self.order {
            if let Some(job) = self.jobs.get(id) {
                if job.status.is_finished() {
                    result.push(job);
                    if result.len() >= limit {
                        return result;
                    }
                }
            }
        }

        result
    }

    /// Number of active jobs
    pub fn active_count(&self) -> usize {
        self.jobs.values().filter(|j| j.status.is_active()).count()
    }

    /// Total number of jobs
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Check if store is empty
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Apply a job event to the store
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
            JobEvent::JobOutput { .. } => {
                // Output is handled separately in log store
            }
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

    /// Prune old completed jobs to stay under the limit
    fn prune_completed(&mut self) {
        let completed_count = self
            .jobs
            .values()
            .filter(|j| j.status.is_finished())
            .count();

        if completed_count <= MAX_COMPLETED_JOBS {
            return;
        }

        // Find oldest completed jobs to remove
        let to_remove: Vec<JobId> = self
            .order
            .iter()
            .rev() // Start from oldest
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

// ============================================================================
// Log prefixing helpers
// ============================================================================

/// Format a log line prefix for a job
pub fn job_log_prefix(job: &Job) -> String {
    if let Some(unit_id) = &job.spec.unit_id {
        format!("{} > {}", unit_id, job.display_name())
    } else {
        format!("job:{}", &job.id[..8.min(job.id.len())])
    }
}

/// Format a log line with job prefix
pub fn format_job_log_line(job: &Job, line: &str) -> String {
    format!("{} | {}", job_log_prefix(job), line)
}
