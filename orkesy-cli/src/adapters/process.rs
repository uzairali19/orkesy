use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use sysinfo::{Pid, System};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, broadcast, mpsc};

#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;

use orkesy_core::adapter::{Adapter, AdapterCommand, AdapterEvent, LogStream};
use orkesy_core::unit::{StopBehavior, StopSignal, Unit, UnitId, UnitMetrics, UnitStatus};

struct ProcessHandle {
    child: Child,
    pgid: i32,
    started_at: std::time::Instant,
}

pub struct ProcessAdapter {
    units: BTreeMap<UnitId, Unit>,
    processes: BTreeMap<UnitId, ProcessHandle>,
    next_id: Arc<AtomicU64>,
    sys: Arc<RwLock<System>>,
    last_metrics: BTreeMap<UnitId, UnitMetrics>,
}

impl ProcessAdapter {
    pub fn new() -> Self {
        Self {
            units: BTreeMap::new(),
            processes: BTreeMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
            sys: Arc::new(RwLock::new(System::new())),
            last_metrics: BTreeMap::new(),
        }
    }

    async fn collect_metrics(&self, pid: u32, uptime_secs: u64) -> UnitMetrics {
        let mut sys = self.sys.write().await;
        sys.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
        );

        if let Some(process) = sys.process(Pid::from_u32(pid)) {
            UnitMetrics {
                cpu_percent: process.cpu_usage(),
                memory_bytes: process.memory(),
                uptime_secs,
                pid: Some(pid),
            }
        } else {
            UnitMetrics {
                cpu_percent: 0.0,
                memory_bytes: 0,
                uptime_secs,
                pid: Some(pid),
            }
        }
    }

    fn emit(&self, event_tx: &broadcast::Sender<AdapterEvent>, event: AdapterEvent) {
        let _ = event_tx.send(event);
    }

    fn emit_log(&self, event_tx: &broadcast::Sender<AdapterEvent>, id: &str, text: String) {
        self.emit(
            event_tx,
            AdapterEvent::LogLine {
                id: id.to_string(),
                stream: LogStream::System,
                text,
            },
        );
    }

    fn emit_status(
        &self,
        event_tx: &broadcast::Sender<AdapterEvent>,
        id: &str,
        status: UnitStatus,
    ) {
        self.emit(
            event_tx,
            AdapterEvent::StatusChanged {
                id: id.to_string(),
                status,
            },
        );
    }

    async fn spawn_unit(
        &mut self,
        id: &UnitId,
        event_tx: &broadcast::Sender<AdapterEvent>,
    ) -> Result<(), String> {
        let unit = self
            .units
            .get(id)
            .ok_or_else(|| format!("unit not found: {}", id))?;

        if unit.start.trim().is_empty() {
            return Err("empty start command".into());
        }

        #[cfg(unix)]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.arg("-c");
            c.arg(&unit.start);
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", &unit.start]);
            c
        };

        if let Some(cwd) = &unit.cwd {
            cmd.current_dir(cwd);
        }

        for (k, v) in &unit.env {
            cmd.env(k, v);
        }

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let pgid = child.id().map(|pid| pid as i32).unwrap_or(-1);

        if let Some(stdout) = child.stdout.take() {
            let tx = event_tx.clone();
            let unit_id = id.clone();
            let next_id = self.next_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = next_id.fetch_add(1, Ordering::SeqCst);
                    let _ = tx.send(AdapterEvent::LogLine {
                        id: unit_id.clone(),
                        stream: LogStream::Stdout,
                        text: line,
                    });
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = event_tx.clone();
            let unit_id = id.clone();
            let next_id = self.next_id.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = next_id.fetch_add(1, Ordering::SeqCst);
                    let _ = tx.send(AdapterEvent::LogLine {
                        id: unit_id.clone(),
                        stream: LogStream::Stderr,
                        text: line,
                    });
                }
            });
        }

        self.processes.insert(
            id.clone(),
            ProcessHandle {
                child,
                pgid,
                started_at: std::time::Instant::now(),
            },
        );

        Ok(())
    }

    async fn stop_unit(&mut self, id: &UnitId, force: bool) -> Result<(), String> {
        let unit = self.units.get(id);
        let stop_behavior = unit
            .map(|u| u.stop.clone())
            .unwrap_or(StopBehavior::Signal(StopSignal::SigTerm));

        if let Some(mut handle) = self.processes.remove(id) {
            self.last_metrics.remove(id);
            match &stop_behavior {
                StopBehavior::Signal(sig) if !force => {
                    #[cfg(unix)]
                    {
                        let signal = match sig {
                            StopSignal::SigInt => libc::SIGINT,
                            StopSignal::SigTerm => libc::SIGTERM,
                            StopSignal::SigKill => libc::SIGKILL,
                        };

                        if handle.pgid > 0 {
                            unsafe {
                                libc::killpg(handle.pgid, signal);
                            }

                            if signal != libc::SIGKILL {
                                tokio::time::sleep(Duration::from_millis(500)).await;

                                if handle.child.try_wait().ok().flatten().is_none() {
                                    unsafe {
                                        libc::killpg(handle.pgid, libc::SIGKILL);
                                    }
                                }
                            }
                        } else {
                            let _ = handle.child.kill().await;
                        }
                    }

                    #[cfg(windows)]
                    {
                        let _ = sig; // Suppress unused warning
                        let _ = handle.child.kill().await;
                    }
                }

                StopBehavior::Command(cmd) if !force => {
                    #[cfg(unix)]
                    let output = Command::new("sh").arg("-c").arg(cmd).output().await;
                    #[cfg(windows)]
                    let output = Command::new("cmd").args(["/C", cmd]).output().await;

                    if let Err(e) = output {
                        #[cfg(unix)]
                        if handle.pgid > 0 {
                            unsafe {
                                libc::killpg(handle.pgid, libc::SIGKILL);
                            }
                        } else {
                            let _ = handle.child.kill().await;
                        }
                        #[cfg(windows)]
                        let _ = handle.child.kill().await;
                        return Err(format!("stop command failed: {}, killed process", e));
                    }
                }

                _ => {
                    #[cfg(unix)]
                    if handle.pgid > 0 {
                        unsafe {
                            libc::killpg(handle.pgid, libc::SIGKILL);
                        }
                    } else {
                        let _ = handle.child.kill().await;
                    }
                    #[cfg(windows)]
                    let _ = handle.child.kill().await;
                }
            }
            Ok(())
        } else {
            Err("process not running".into())
        }
    }

    async fn install_unit(
        &self,
        id: &UnitId,
        event_tx: &broadcast::Sender<AdapterEvent>,
    ) -> Result<(), String> {
        let unit = self
            .units
            .get(id)
            .ok_or_else(|| format!("unit not found: {}", id))?;

        if unit.install.is_empty() {
            self.emit_log(event_tx, id, "no install commands defined".into());
            return Ok(());
        }

        for cmd in &unit.install {
            self.emit_log(event_tx, id, format!("$ {}", cmd));

            #[cfg(unix)]
            let mut command = {
                let mut c = Command::new("sh");
                c.arg("-c").arg(cmd);
                c
            };
            #[cfg(windows)]
            let mut command = {
                let mut c = Command::new("cmd");
                c.args(["/C", cmd]);
                c
            };

            if let Some(cwd) = &unit.cwd {
                command.current_dir(cwd);
            }

            for (k, v) in &unit.env {
                command.env(k, v);
            }

            let output = command.output().await.map_err(|e| e.to_string())?;

            if !output.stdout.is_empty() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    for line in s.lines() {
                        self.emit(
                            event_tx,
                            AdapterEvent::LogLine {
                                id: id.clone(),
                                stream: LogStream::Stdout,
                                text: line.to_string(),
                            },
                        );
                    }
                }
            }

            if !output.stderr.is_empty() {
                if let Ok(s) = String::from_utf8(output.stderr) {
                    for line in s.lines() {
                        self.emit(
                            event_tx,
                            AdapterEvent::LogLine {
                                id: id.clone(),
                                stream: LogStream::Stderr,
                                text: line.to_string(),
                            },
                        );
                    }
                }
            }

            if !output.status.success() {
                return Err(format!(
                    "install command failed with exit code: {:?}",
                    output.status.code()
                ));
            }
        }

        self.emit_log(event_tx, id, "install completed".into());
        Ok(())
    }
}

impl Default for ProcessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

#[async_trait]
impl Adapter for ProcessAdapter {
    fn name(&self) -> &'static str {
        "process"
    }

    async fn run(
        &mut self,
        mut command_rx: mpsc::Receiver<AdapterCommand>,
        event_tx: broadcast::Sender<AdapterEvent>,
        units: Vec<Unit>,
    ) {
        for unit in units {
            self.units.insert(unit.id.clone(), unit);
        }

        let mut check_interval = tokio::time::interval(Duration::from_millis(100));
        check_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut metrics_interval = tokio::time::interval(Duration::from_secs(2));
        metrics_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = check_interval.tick() => {
                    let mut exited = vec![];
                    for (id, handle) in &mut self.processes {
                        if let Ok(Some(status)) = handle.child.try_wait() {
                            exited.push((id.clone(), status.code()));
                        }
                    }

                    for (id, code) in exited {
                        self.processes.remove(&id);
                        self.last_metrics.remove(&id);
                        self.emit_status(&event_tx, &id, UnitStatus::Exited { code });
                        self.emit_log(&event_tx, &id, format!("process exited with code: {:?}", code));
                    }
                }

                _ = metrics_interval.tick() => {
                    for (id, handle) in &self.processes {
                        if let Some(pid) = handle.child.id() {
                            let uptime = handle.started_at.elapsed().as_secs();
                            let metrics = self.collect_metrics(pid, uptime).await;
                            self.last_metrics.insert(id.clone(), metrics.clone());
                            self.emit(&event_tx, AdapterEvent::MetricsUpdated {
                                id: id.clone(),
                                metrics,
                            });
                        }
                    }
                }

                cmd = command_rx.recv() => {
                    let Some(cmd) = cmd else { break };

                    match cmd {
                        AdapterCommand::Shutdown => {
                            let ids: Vec<_> = self.processes.keys().cloned().collect();
                            for id in ids {
                                let _ = self.stop_unit(&id, false).await;
                            }
                            break;
                        }

                        AdapterCommand::Start { id } => {
                            if self.processes.contains_key(&id) {
                                self.emit_log(&event_tx, &id, "[warn] already running".into());
                                continue;
                            }

                            self.emit_status(&event_tx, &id, UnitStatus::Starting);

                            match self.spawn_unit(&id, &event_tx).await {
                                Ok(()) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Running);
                                }
                                Err(e) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Errored { message: e.clone() });
                                    self.emit_log(&event_tx, &id, format!("[error] {}", e));
                                }
                            }
                        }

                        AdapterCommand::Stop { id } => {
                            self.emit_log(&event_tx, &id, "stopping...".into());
                            self.emit_status(&event_tx, &id, UnitStatus::Stopping);

                            match self.stop_unit(&id, false).await {
                                Ok(()) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                                }
                                Err(e) => {
                                    self.emit_log(&event_tx, &id, format!("[warn] {}", e));
                                }
                            }
                        }

                        AdapterCommand::Restart { id } => {
                            self.emit_log(&event_tx, &id, "restarting...".into());

                            let _ = self.stop_unit(&id, false).await;
                            tokio::time::sleep(Duration::from_millis(100)).await;

                            self.emit_status(&event_tx, &id, UnitStatus::Starting);
                            match self.spawn_unit(&id, &event_tx).await {
                                Ok(()) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Running);
                                    self.emit_log(&event_tx, &id, "restarted".into());
                                }
                                Err(e) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Errored { message: e.clone() });
                                    self.emit_log(&event_tx, &id, format!("[error] restart failed: {}", e));
                                }
                            }
                        }

                        AdapterCommand::Kill { id } => {
                            match self.stop_unit(&id, true).await {
                                Ok(()) => {
                                    self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                                    self.emit_log(&event_tx, &id, "killed".into());
                                }
                                Err(e) => {
                                    self.emit_log(&event_tx, &id, format!("[warn] {}", e));
                                }
                            }
                        }

                        AdapterCommand::Toggle { id } => {
                            if self.processes.contains_key(&id) {
                                self.emit_log(&event_tx, &id, "stopping...".into());
                                self.emit_status(&event_tx, &id, UnitStatus::Stopping);
                                let _ = self.stop_unit(&id, false).await;
                                self.emit_status(&event_tx, &id, UnitStatus::Stopped);
                            } else {
                                self.emit_status(&event_tx, &id, UnitStatus::Starting);
                                match self.spawn_unit(&id, &event_tx).await {
                                    Ok(()) => {
                                        self.emit_status(&event_tx, &id, UnitStatus::Running);
                                    }
                                    Err(e) => {
                                        self.emit_status(&event_tx, &id, UnitStatus::Errored { message: e.clone() });
                                        self.emit_log(&event_tx, &id, format!("[error] {}", e));
                                    }
                                }
                            }
                        }

                        AdapterCommand::ClearLogs { id } => {
                            self.emit_log(&event_tx, &id, "logs cleared".into());
                        }

                        AdapterCommand::Install { id } => {
                            self.emit_log(&event_tx, &id, "installing dependencies...".into());
                            match self.install_unit(&id, &event_tx).await {
                                Ok(()) => {}
                                Err(e) => {
                                    self.emit_log(&event_tx, &id, format!("[error] install failed: {}", e));
                                }
                            }
                        }

                        AdapterCommand::Exec { id, cmd } => {
                            let shown = cmd.join(" ");
                            self.emit_log(&event_tx, &id, format!("$ {}", shown));

                            if cmd.is_empty() {
                                self.emit_log(&event_tx, &id, "[error] empty command".into());
                                continue;
                            }

                            let unit = self.units.get(&id);
                            let mut command = Command::new(&cmd[0]);
                            command.args(&cmd[1..]);

                            if let Some(u) = unit {
                                if let Some(cwd) = &u.cwd {
                                    command.current_dir(cwd);
                                }
                                for (k, v) in &u.env {
                                    command.env(k, v);
                                }
                            }

                            let output = command.output().await;

                            match output {
                                Ok(out) => {
                                    if !out.stdout.is_empty() {
                                        if let Ok(s) = String::from_utf8(out.stdout) {
                                            for line in s.lines() {
                                                self.emit(
                                                    &event_tx,
                                                    AdapterEvent::LogLine {
                                                        id: id.clone(),
                                                        stream: LogStream::Stdout,
                                                        text: line.to_string(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    if !out.stderr.is_empty() {
                                        if let Ok(s) = String::from_utf8(out.stderr) {
                                            for line in s.lines() {
                                                self.emit(
                                                    &event_tx,
                                                    AdapterEvent::LogLine {
                                                        id: id.clone(),
                                                        stream: LogStream::Stderr,
                                                        text: line.to_string(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    let status = if out.status.success() {
                                        "ok".to_string()
                                    } else {
                                        format!("exit code: {:?}", out.status.code())
                                    };
                                    self.emit_log(&event_tx, &id, status);
                                }
                                Err(e) => {
                                    self.emit_log(&event_tx, &id, format!("[error] {}", e));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn status(&self, id: &str) -> Option<UnitStatus> {
        if self.processes.contains_key(id) {
            Some(UnitStatus::Running)
        } else if self.units.contains_key(id) {
            Some(UnitStatus::Stopped)
        } else {
            None
        }
    }

    fn metrics(&self, id: &str) -> Option<UnitMetrics> {
        if let Some(cached) = self.last_metrics.get(id) {
            return Some(cached.clone());
        }
        self.processes.get(id).map(|handle| UnitMetrics {
            cpu_percent: 0.0,
            memory_bytes: 0,
            uptime_secs: handle.started_at.elapsed().as_secs(),
            pid: handle.child.id(),
        })
    }
}
