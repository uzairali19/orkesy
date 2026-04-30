use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use orkesy_core::config::LogHistoryConfig;
use orkesy_core::reducer::{EventEnvelope, RuntimeEvent};
use orkesy_core::state::LogStream;
use tokio::sync::broadcast;

const FORMAT_VERSION: u32 = 1;
const FLUSH_EVERY_LINES: usize = 1_000;
const FLUSH_EVERY_SECS: u64 = 5;

pub fn state_root() -> PathBuf {
    if let Ok(p) = std::env::var("ORKESY_STATE_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(p).join("orkesy");
    }
    if let Ok(home) = std::env::var("HOME") {
        let xdg = PathBuf::from(&home).join(".local/state/orkesy");
        if xdg.parent().map(|p| p.exists()).unwrap_or(false) {
            return xdg;
        }
        return PathBuf::from(home).join(".orkesy");
    }
    PathBuf::from(".orkesy")
}

// FNV-1a 64-bit. We don't need cryptographic strength here — this is just a
// stable directory name — and avoiding `sha2` keeps the dep tree small.
pub fn project_hash(project_root: &Path) -> String {
    let bytes = project_root.to_string_lossy();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{:016x}", h)[..12].to_string()
}

pub fn project_dir(project_root: &Path) -> PathBuf {
    state_root()
        .join("projects")
        .join(project_hash(project_root))
}

fn ensure_meta(project_root: &Path, project_name: Option<&str>) -> std::io::Result<()> {
    let dir = project_dir(project_root);
    fs::create_dir_all(dir.join("logs"))?;
    let meta_path = dir.join("meta.json");
    if meta_path.exists() {
        return Ok(());
    }
    let meta = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "project_name": project_name.unwrap_or(""),
        "project_root": project_root.to_string_lossy(),
        "created_at": now_secs(),
    });
    fs::write(
        meta_path,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Howard Hinnant's civil_from_days. Hand-rolled to avoid pulling `chrono` /
// `time` for one calendar conversion. Proleptic Gregorian, UTC.
fn utc_ymdh(t: SystemTime) -> (i32, u32, u32, u32) {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hour = (sod / 3600) as u32;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = if m <= 2 { y + 1 } else { y };
    (yy as i32, m as u32, d as u32, hour)
}

fn hour_filename(t: SystemTime) -> String {
    let (y, mo, d, h) = utc_ymdh(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}.jsonl")
}

pub fn spawn_writer(
    cfg: &LogHistoryConfig,
    project_root: PathBuf,
    project_name: Option<String>,
    mut events: broadcast::Receiver<EventEnvelope>,
) {
    if !cfg.enabled {
        return;
    }
    if let Err(e) = ensure_meta(&project_root, project_name.as_deref()) {
        eprintln!(
            "log_history: cannot prepare {}: {e} (history disabled for this session)",
            project_dir(&project_root).display()
        );
        return;
    }

    let logs_dir = project_dir(&project_root).join("logs");

    tokio::spawn(async move {
        let mut current_bucket: Option<String> = None;
        let mut writer: Option<BufWriter<File>> = None;
        let mut lines_since_flush = 0usize;
        let mut last_flush = std::time::Instant::now();
        let mut dropped_since_warn: u64 = 0;

        loop {
            match events.recv().await {
                Ok(env) => {
                    if let Some(line) = render_event(&env) {
                        let bucket = hour_filename(env.at);
                        if current_bucket.as_deref() != Some(bucket.as_str()) {
                            if let Some(mut w) = writer.take() {
                                let _ = w.flush();
                            }
                            let path = logs_dir.join(&bucket);
                            match open_append(&path) {
                                Ok(f) => writer = Some(BufWriter::new(f)),
                                Err(e) => {
                                    eprintln!("log_history: cannot open {}: {e}", path.display());
                                    writer = None;
                                }
                            }
                            current_bucket = Some(bucket);
                            lines_since_flush = 0;
                            last_flush = std::time::Instant::now();
                        }

                        if let Some(w) = writer.as_mut() {
                            if let Err(e) = writeln!(w, "{line}") {
                                eprintln!("log_history: write failed: {e}");
                            } else {
                                lines_since_flush += 1;
                            }
                        }

                        if lines_since_flush >= FLUSH_EVERY_LINES
                            || last_flush.elapsed().as_secs() >= FLUSH_EVERY_SECS
                        {
                            if let Some(w) = writer.as_mut() {
                                let _ = w.flush();
                            }
                            lines_since_flush = 0;
                            last_flush = std::time::Instant::now();
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    dropped_since_warn += n;
                    if last_flush.elapsed().as_secs() >= 60 {
                        eprintln!(
                            "log_history: dropped {dropped_since_warn} events (writer is lagging)"
                        );
                        dropped_since_warn = 0;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }

        if let Some(mut w) = writer {
            let _ = w.flush();
        }
    });
}

fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct HistoryRecord {
    pub t: f64,
    #[serde(rename = "u")]
    pub unit: String,
    #[serde(rename = "s")]
    pub stream: String,
    #[serde(rename = "l")]
    pub level: String,
    #[serde(rename = "m")]
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct ReadQuery {
    pub unit: Option<String>,
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub level: LevelFilter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LevelFilter {
    #[default]
    All,
    WarnAndAbove,
    ErrorOnly,
}

impl LevelFilter {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "all" | "" => Ok(LevelFilter::All),
            "warn" | "warning" => Ok(LevelFilter::WarnAndAbove),
            "error" | "err" => Ok(LevelFilter::ErrorOnly),
            other => Err(format!(
                "unknown level filter '{other}' (expected: all, warn, error)"
            )),
        }
    }

    fn matches(&self, level: &str) -> bool {
        match self {
            LevelFilter::All => true,
            LevelFilter::WarnAndAbove => matches!(level, "warn" | "error"),
            LevelFilter::ErrorOnly => level == "error",
        }
    }
}

pub fn parse_duration(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let (num, suffix) = s.split_at(s.len() - 1);
    let n: u64 = num
        .parse()
        .map_err(|_| format!("invalid duration '{s}' (expected NNs/m/h/d)"))?;
    match suffix {
        "s" => Ok(n),
        "m" => Ok(n * 60),
        "h" => Ok(n * 3600),
        "d" => Ok(n * 86_400),
        _ => Err(format!("invalid duration suffix '{suffix}' (use s/m/h/d)")),
    }
}

pub fn read_history(project_root: &Path, query: &ReadQuery) -> std::io::Result<Vec<HistoryRecord>> {
    let logs_dir = project_dir(project_root).join("logs");
    if !logs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut hour_files: Vec<PathBuf> = fs::read_dir(&logs_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    hour_files.sort();

    let mut out = Vec::new();
    for path in hour_files {
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("log_history: skipping {} ({e})", path.display());
                continue;
            }
        };
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            // Corrupt lines are skipped silently — see ARCHITECTURE.md.
            let rec: HistoryRecord = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => continue,
            };
            if let Some(from) = query.from
                && rec.t < from
            {
                continue;
            }
            if let Some(to) = query.to
                && rec.t >= to
            {
                continue;
            }
            if let Some(unit) = &query.unit
                && rec.unit != *unit
            {
                continue;
            }
            if !query.level.matches(&rec.level) {
                continue;
            }
            out.push(rec);
        }
    }

    Ok(out)
}

pub fn format_text(rec: &HistoryRecord) -> String {
    let secs = rec.t as u64;
    let (y, mo, d, h) = utc_ymdh(UNIX_EPOCH + std::time::Duration::from_secs(secs));
    let m = (secs / 60) % 60;
    let s = secs % 60;
    let level_tag = match rec.level.as_str() {
        "error" => "[ERR ]",
        "warn" => "[WARN]",
        "info" => "[INFO]",
        "debug" => "[DBG ]",
        _ => "[----]",
    };
    format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02} {level_tag} {} | {}",
        rec.unit, rec.message
    )
}

fn render_event(env: &EventEnvelope) -> Option<String> {
    let RuntimeEvent::LogLine { id, stream, text } = &env.event else {
        return None;
    };
    let t = env
        .at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let stream_label = match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        LogStream::System => "system",
    };
    let level = orkesy_core::log_filter::detect_level(text);
    let level_label = match level {
        orkesy_core::log_filter::LogLevel::Debug => "debug",
        orkesy_core::log_filter::LogLevel::Info => "info",
        orkesy_core::log_filter::LogLevel::Warn => "warn",
        orkesy_core::log_filter::LogLevel::Error => "error",
    };
    let value = serde_json::json!({
        "t": t,
        "u": id,
        "s": stream_label,
        "l": level_label,
        "m": text,
    });
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_hash_is_stable() {
        let a = project_hash(Path::new("/tmp/foo"));
        let b = project_hash(Path::new("/tmp/foo"));
        let c = project_hash(Path::new("/tmp/bar"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 12);
    }

    #[test]
    fn hour_filename_is_utc_iso_like() {
        // 2024-01-01T00:00:00 UTC = 1_704_067_200 seconds since epoch.
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_704_067_200);
        assert_eq!(hour_filename(t), "2024-01-01T00.jsonl");

        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_704_067_200 + 3_600);
        assert_eq!(hour_filename(t), "2024-01-01T01.jsonl");
        assert!(hour_filename(t).is_ascii());
    }

    #[test]
    fn render_event_emits_jsonl_for_log_lines() {
        let env = EventEnvelope {
            id: 1,
            at: UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            event: RuntimeEvent::LogLine {
                id: "api".into(),
                stream: LogStream::Stderr,
                text: "ERROR: connect refused".into(),
            },
        };
        let line = render_event(&env).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["u"], "api");
        assert_eq!(v["s"], "stderr");
        assert_eq!(v["l"], "error");
        assert_eq!(v["m"], "ERROR: connect refused");
    }

    #[test]
    fn render_event_skips_non_log_events() {
        let env = EventEnvelope {
            id: 1,
            at: UNIX_EPOCH,
            event: RuntimeEvent::ClearLogs { id: "api".into() },
        };
        assert!(render_event(&env).is_none());
    }

    #[test]
    fn parse_duration_handles_common_suffixes() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("15m").unwrap(), 15 * 60);
        assert_eq!(parse_duration("1h").unwrap(), 3600);
        assert_eq!(parse_duration("7d").unwrap(), 7 * 86_400);
        assert!(parse_duration("").is_err());
        assert!(parse_duration("1y").is_err());
        assert!(parse_duration("xh").is_err());
    }

    #[test]
    fn level_filter_parses_synonyms() {
        assert_eq!(LevelFilter::parse("error").unwrap(), LevelFilter::ErrorOnly);
        assert_eq!(LevelFilter::parse("err").unwrap(), LevelFilter::ErrorOnly);
        assert_eq!(
            LevelFilter::parse("warn").unwrap(),
            LevelFilter::WarnAndAbove
        );
        assert_eq!(LevelFilter::parse("all").unwrap(), LevelFilter::All);
        assert!(LevelFilter::parse("xyz").is_err());
    }

    #[test]
    fn read_history_round_trip() {
        let tmp =
            std::env::temp_dir().join(format!("orkesy-test-{}-{}", std::process::id(), now_secs()));
        std::fs::create_dir_all(&tmp).unwrap();

        // Redirect the state root so we don't pollute the user's actual one.
        unsafe { std::env::set_var("ORKESY_STATE_DIR", tmp.to_str().unwrap()) };

        let project_root = tmp.join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let logs_dir = project_dir(&project_root).join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();

        let t0 = 1_704_067_200.0_f64;
        let recs = [
            r#"{"t":1704067200.0,"u":"api","s":"stdout","l":"info","m":"hello"}"#,
            r#"{"t":1704067201.0,"u":"api","s":"stderr","l":"error","m":"boom"}"#,
            r#"{"t":1704067202.0,"u":"worker","s":"stdout","l":"info","m":"tick"}"#,
        ];
        let file = logs_dir.join("2024-01-01T00.jsonl");
        std::fs::write(&file, recs.join("\n")).unwrap();

        let all = read_history(
            &project_root,
            &ReadQuery {
                level: LevelFilter::All,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.len(), 3);

        let api_only = read_history(
            &project_root,
            &ReadQuery {
                unit: Some("api".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(api_only.len(), 2);

        let errors_only = read_history(
            &project_root,
            &ReadQuery {
                level: LevelFilter::ErrorOnly,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(errors_only.len(), 1);
        assert_eq!(errors_only[0].message, "boom");

        let after_t1 = read_history(
            &project_root,
            &ReadQuery {
                from: Some(t0 + 1.5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(after_t1.len(), 1);
        assert_eq!(after_t1[0].message, "tick");

        unsafe { std::env::remove_var("ORKESY_STATE_DIR") };
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
