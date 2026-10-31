use crate::engine::Usage;
use crate::now_utc;
use crate::runner::{CallLog, write_atomic};
use crate::state::{state_name, step_name, waiting_on};
use fabr_core::FabrError;
use fabr_core::redact::redact;
use fabr_core::taskrec::{Step, TaskRecord, registry_filename};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Model,
    Tool,
    Check,
    Parked,
    Retry,
    Queued,
    Paused,
}

impl Activity {
    pub fn name(self) -> &'static str {
        match self {
            Activity::Model => "model",
            Activity::Tool => "tool",
            Activity::Check => "check",
            Activity::Parked => "parked",
            Activity::Retry => "retry",
            Activity::Queued => "queued",
            Activity::Paused => "paused",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub activity: Option<Activity>,
    pub retry_in_s: Option<u64>,
    pub last_error: Option<String>,
}

impl Extras {
    pub fn doing(activity: Activity) -> Extras {
        Extras { activity: Some(activity), ..Extras::default() }
    }
}

struct Live {
    task_path: String,
    task_id: String,
    folder: String,
    state: &'static str,
    step: Option<Step>,
    step_at: String,
    started_at: String,
    waiting: Option<&'static str>,
    extras: Extras,
    cost_usd: f64,
    log_tail: Vec<String>,
    question: String,
}

pub struct Board {
    space: PathBuf,
    hb_interval_s: u64,
    calls: Option<Arc<CallLog>>,
    live: Mutex<HashMap<PathBuf, Live>>,
}

impl Board {
    pub fn new(space: &Path, hb_interval_s: u64, calls: Option<Arc<CallLog>>) -> Arc<Board> {
        Arc::new(Board { space: space.to_path_buf(), hb_interval_s: hb_interval_s.max(1), calls, live: Mutex::new(HashMap::new()) })
    }

    pub fn space(&self) -> &Path {
        &self.space
    }

    pub fn hb_interval_s(&self) -> u64 {
        self.hb_interval_s
    }

    pub fn file_for(&self, task_id: &str, task_path: &str) -> PathBuf {
        self.space.join("registry").join(registry_filename(task_id, task_path))
    }

    pub fn publish(&self, node_dir: &Path, record_path: &Path, rec: &TaskRecord, extras: Extras) -> Result<(), FabrError> {
        let task_path = crate::task_path_key(record_path);
        let file = self.file_for(&rec.id, &task_path);
        let now = now_utc();
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        let (started_at, step_at) = match live.get(&file) {
            Some(prev) => (prev.started_at.clone(), if prev.step == rec.step { prev.step_at.clone() } else { now.clone() }),
            None => {
                let disk = read_times(&file);
                let started = disk.as_ref().map(|d| d.0.clone()).unwrap_or_else(|| now.clone());
                let step_at = match &disk {
                    Some((_, step, at)) if step.as_deref() == rec.step.map(step_name) => at.clone(),
                    _ => now.clone(),
                };
                (started, step_at)
            }
        };
        let folder = node_dir.canonicalize().unwrap_or_else(|_| node_dir.to_path_buf());
        let entry = Live {
            task_path,
            task_id: rec.id.clone(),
            folder: folder.to_string_lossy().into_owned(),
            state: state_name(rec.state),
            step: rec.step,
            step_at,
            started_at,
            waiting: waiting_on(rec),
            extras: Extras { last_error: extras.last_error.map(|e| redact(&e)), ..extras },
            cost_usd: rec.cost_usd,
            log_tail: log_tail(&rec.log),
            question: match &rec.question {
                None => "null".into(),
                Some(q) => serde_json::to_string(q).unwrap_or_else(|_| "null".into()),
            },
        };
        self.write(&file, &entry, &now)?;
        live.insert(file, entry);
        Ok(())
    }

    pub fn spent(&self, task_id: &str, step: &str, session: Option<&str>, usage: &Usage, usd: f64) {
        if let Some(calls) = &self.calls {
            calls.record(task_id, step, session, usage, usd);
        }
    }

    pub fn remove(&self, task_id: &str, task_path: &str) {
        let file = self.file_for(task_id, task_path);
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.remove(&file);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("remove registry {}: {e}", file.display()),
        }
        if let Some(calls) = &self.calls {
            calls.forget(task_id);
        }
    }

    pub fn beat(&self) {
        let now = now_utc();
        let live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        for (file, entry) in live.iter() {
            if let Err(e) = self.write(file, entry, &now) {
                log::warn!("registry refresh {}: {}", file.display(), e.message());
            }
        }
    }

    pub fn clear(&self) {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        for file in live.keys() {
            let _ = std::fs::remove_file(file);
        }
        live.clear();
    }

    fn write(&self, file: &Path, e: &Live, now: &str) -> Result<(), FabrError> {
        let mut body = String::new();
        let q = yaml_quote;
        let _ = writeln!(body, "task_path: {}", q(&e.task_path));
        let _ = writeln!(body, "task_id: {}", q(&e.task_id));
        let _ = writeln!(body, "folder: {}", q(&e.folder));
        let _ = writeln!(body, "state: {}", e.state);
        let _ = writeln!(body, "step: {}", e.step.map_or("null", step_name));
        let _ = writeln!(body, "step_at: {}", q(&e.step_at));
        let _ = writeln!(body, "started_at: {}", q(&e.started_at));
        let _ = writeln!(body, "heartbeat_at: {}", q(now));
        let _ = writeln!(body, "heartbeat_interval_s: {}", self.hb_interval_s);
        let _ = writeln!(body, "waiting: {}", e.waiting.unwrap_or("null"));
        let _ = writeln!(body, "activity: {}", e.extras.activity.map_or("null", Activity::name));
        let _ = writeln!(body, "retry_in_s: {}", e.extras.retry_in_s.map_or("null".to_string(), |n| n.to_string()));
        let _ = writeln!(body, "last_error: {}", e.extras.last_error.as_deref().map_or("null".to_string(), q));
        let _ = writeln!(body, "cost_usd: {}", format_cost(e.cost_usd));
        let usage = self.calls.as_ref().map(|c| c.usage(&e.task_id)).unwrap_or_default();
        if usage.is_empty() {
            body.push_str("usage: {}\n");
        } else {
            body.push_str("usage:\n");
            for (step, u) in usage {
                let _ = writeln!(
                    body,
                    "  {step}: {{calls: {}, input_tokens: {}, output_tokens: {}, usd: {}}}",
                    u.calls,
                    u.input_tokens,
                    u.output_tokens,
                    format_cost(u.usd)
                );
            }
        }
        if e.log_tail.is_empty() {
            body.push_str("log_tail: []\n");
        } else {
            body.push_str("log_tail:\n");
            for line in &e.log_tail {
                let _ = writeln!(body, "  - {}", q(line));
            }
        }
        let _ = writeln!(body, "question: {}", e.question);
        let _ = writeln!(body, "pid: {}", std::process::id());
        write_atomic(file, &body).map_err(|err| FabrError::Io(format!("{}: {err}", file.display())))
    }
}

fn read_times(file: &Path) -> Option<(String, Option<String>, String)> {
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let text = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Some((text("started_at")?, text("step"), text("step_at").or_else(|| text("started_at"))?))
}

fn log_tail(log: &str) -> Vec<String> {
    let lines: Vec<String> = log.lines().filter(|l| !l.trim().is_empty()).map(|l| l.strip_prefix("- ").unwrap_or(l).to_string()).collect();
    let start = lines.len().saturating_sub(20);
    lines[start..].to_vec()
}

fn format_cost(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

fn yaml_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::heartbeat::{Beating, Heartbeat};

    #[test]
    fn the_heartbeat_thread_keeps_entries_fresh_while_the_runner_thread_is_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let space = dir.path().join("space");
        let node = dir.path().join("node");
        let record = node.join(".fabric-space/tasks/t.md");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        let mut rec = crate::link::test_reminder("t");
        rec.id = "t".into();
        let board = Board::new(&space, 1, None);
        board.publish(&node, &record, &rec, Extras::doing(Activity::Model)).unwrap();
        let file = board.file_for("t", &crate::task_path_key(&record));
        let beat = |f: &Path| {
            let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            doc["heartbeat_at"].as_str().unwrap().to_string()
        };
        let first = beat(&file);
        let beating = Beating::spawn(Heartbeat::new(&space, 1), board.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2400));
        let later = beat(&file);
        beating.stop(true);
        assert!(later > first, "{first} {later}");
        let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["activity"].as_str(), Some("model"));
        assert_eq!(doc["pid"].as_u64(), Some(std::process::id() as u64));
    }

    #[test]
    fn a_removed_entry_is_not_brought_back_by_the_heartbeat() {
        let dir = tempfile::tempdir().unwrap();
        let space = dir.path().join("space");
        let record = dir.path().join("node/.fabric-space/tasks/t.md");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        let mut rec = crate::link::test_reminder("t");
        rec.id = "t".into();
        let board = Board::new(&space, 1, None);
        board.publish(dir.path(), &record, &rec, Extras::default()).unwrap();
        board.remove("t", &crate::task_path_key(&record));
        board.beat();
        assert!(!board.file_for("t", &crate::task_path_key(&record)).exists());
    }
}
