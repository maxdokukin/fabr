use crate::error::FabrError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Reminder,
    Task,
    Defined,
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Gather,
    Define,
    Plan,
    Approve,
    Execute,
    Evaluate,
    Learn,
    Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Gui,
    Telegram,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    Reversible,
    Irreversible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictResult {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    Open,
    Choice,
    Grant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Task,
    Project,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSource {
    pub node_path: String,
    pub todo_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Requests {
    pub delegate: bool,
    pub cancel: bool,
    pub redelegate: bool,
    #[serde(default)]
    pub pause: bool,
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub redo: Option<RedoStage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedoStage {
    Define,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    Human,
    Agent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct By {
    #[serde(default)]
    pub context: Option<Author>,
    #[serde(default)]
    pub spec: Option<Author>,
    #[serde(default)]
    pub success: Option<Author>,
    #[serde(default)]
    pub plan: Option<Author>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRequires {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    pub field: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grants {
    #[serde(default)]
    pub requested: Vec<String>,
    #[serde(default)]
    pub granted: Vec<Grant>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prior {
    pub text: String,
    #[serde(default)]
    pub source_task: Option<String>,
    #[serde(default)]
    pub last_confirmed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub granted: Option<Vec<Grant>>,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub prior: Option<Prior>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub asked_at: Option<String>,
    #[serde(default)]
    pub answer: Option<Answer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assumption {
    pub rule_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One concrete operation covered by the approval digest.
///
/// Absent on records that predate parameter binding. An empty list is omitted
/// from the digest so those approvals keep the same hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundAction {
    pub tool: String,
    /// Canonical JSON of the resolved parameters.
    pub params: String,
    #[serde(default)]
    pub destination: String,
    #[serde(default)]
    pub effect: String,
    #[serde(default)]
    pub grant_class: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanMeta {
    #[serde(default)]
    pub reversibility: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub requires: PlanRequires,
    /// Included in [`approval_digest`] only when non-empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bound_actions: Vec<BoundAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assertion {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub status: String,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictItem {
    pub id: String,
    pub pass: bool,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub result: String,
    pub at: String,
    #[serde(default)]
    pub items: Vec<VerdictItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Links {
    #[serde(default)]
    pub redelegated_from: Option<String>,
    #[serde(default)]
    pub redelegated_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GuiPatch {
    #[serde(default)]
    pub requests: Option<Requests>,
    #[serde(default)]
    pub approval: Option<Approval>,
    #[serde(default)]
    pub grants_granted: Option<Vec<Grant>>,
    #[serde(default)]
    pub answer: Option<Answer>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrontMatter {
    pub schema: i64,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub pri: u8,
    pub state: State,
    #[serde(default)]
    pub step: Option<Step>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub suspended: bool,
    #[serde(default)]
    pub suspend_until: Option<String>,
    #[serde(default)]
    pub source: Option<TaskSource>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    pub requests: Requests,
    #[serde(default)]
    pub question: Option<Question>,
    #[serde(default)]
    pub answers: Vec<Question>,
    pub grants: Grants,
    #[serde(default)]
    pub plan_meta: Option<PlanMeta>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub approval: Option<Approval>,
    #[serde(default)]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub cost_usd: f64,
    pub links: Links,
    #[serde(default)]
    pub by: By,
    #[serde(default)]
    pub needs: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRecord {
    pub schema: i64,
    pub id: String,
    pub source: Option<TaskSource>,
    pub title: String,
    pub due: Option<String>,
    pub pri: u8,
    pub state: State,
    pub step: Option<Step>,
    pub waiting: bool,
    pub paused: bool,
    pub suspended: bool,
    pub suspend_until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub requests: Requests,
    pub question: Option<Question>,
    pub answers: Vec<Question>,
    pub grants: Grants,
    pub plan_meta: Option<PlanMeta>,
    pub assertions: Vec<Assertion>,
    pub approval: Option<Approval>,
    pub verdict: Option<Verdict>,
    pub cost_usd: f64,
    pub links: Links,
    pub by: By,
    pub needs: Option<String>,
    pub context: String,
    pub spec: String,
    pub success: String,
    pub plan: String,
    pub log: String,
}

pub const SCHEMA: i64 = 6;

pub fn valid_due(s: &str) -> bool {
    match s.len() {
        10 => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok(),
        16 => chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").is_ok(),
        _ => false,
    }
}

const SECTIONS: [&str; 5] = ["Context", "Spec", "Success", "Plan", "Log"];

pub fn parse(text: &str) -> Result<TaskRecord, FabrError> {
    let rest = text.strip_prefix("---").ok_or_else(|| FabrError::Unprocessable("record has no front matter".into()))?;
    let end = rest.find("\n---").ok_or_else(|| FabrError::Unprocessable("front matter is not closed".into()))?;
    let yaml = &rest[..end];
    let body = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    let fm: FrontMatter = serde_yaml::from_str(yaml).map_err(|e| FabrError::Unprocessable(format!("front matter: {e}")))?;
    if !matches!(fm.schema, 2..=6) {
        return Err(FabrError::Unprocessable(format!("unsupported record schema {}", fm.schema)));
    }
    if fm.pri > 3 {
        return Err(FabrError::Unprocessable(format!("pri must be 0-3, got {}", fm.pri)));
    }
    let mut sections = split_sections(body);
    Ok(TaskRecord {
        schema: SCHEMA,
        id: fm.id,
        source: fm.source,
        title: fm.title,
        due: fm.due,
        pri: fm.pri,
        state: fm.state,
        step: fm.step,
        waiting: fm.waiting,
        paused: fm.paused,
        suspended: fm.suspended,
        suspend_until: fm.suspend_until,
        created_at: fm.created_at,
        updated_at: fm.updated_at,
        requests: fm.requests,
        question: fm.question,
        answers: fm.answers,
        grants: fm.grants,
        plan_meta: fm.plan_meta,
        assertions: fm.assertions,
        approval: fm.approval,
        verdict: fm.verdict,
        cost_usd: fm.cost_usd,
        links: fm.links,
        by: fm.by,
        needs: fm.needs,
        context: sections.remove("Context").unwrap_or_default(),
        spec: sections.remove("Spec").unwrap_or_default(),
        success: sections.remove("Success").unwrap_or_default(),
        plan: sections.remove("Plan").unwrap_or_default(),
        log: sections.remove("Log").unwrap_or_default(),
    })
}

fn split_sections(body: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut current: Option<String> = None;
    let mut buf = String::new();
    for line in body.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            if let Some(key) = current.take() {
                out.insert(key, buf.trim_matches('\n').to_string());
            }
            buf = String::new();
            current = Some(name.trim().to_string());
        } else if current.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(key) = current.take() {
        out.insert(key, buf.trim_matches('\n').to_string());
    }
    out
}

pub fn render(rec: &TaskRecord) -> String {
    let fm = FrontMatter {
        schema: SCHEMA,
        id: rec.id.clone(),
        title: rec.title.clone(),
        due: rec.due.clone(),
        pri: rec.pri,
        state: rec.state,
        step: rec.step,
        waiting: rec.waiting,
        paused: rec.paused,
        suspended: rec.suspended,
        suspend_until: rec.suspend_until.clone(),
        source: rec.source.clone(),
        created_at: rec.created_at.clone(),
        updated_at: rec.updated_at.clone(),
        requests: rec.requests.clone(),
        question: rec.question.clone(),
        answers: rec.answers.clone(),
        grants: rec.grants.clone(),
        plan_meta: rec.plan_meta.clone(),
        assertions: rec.assertions.clone(),
        approval: rec.approval.clone(),
        verdict: rec.verdict.clone(),
        cost_usd: rec.cost_usd,
        links: rec.links.clone(),
        by: rec.by.clone(),
        needs: rec.needs.clone(),
    };
    let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    let mut out = String::from("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    for name in SECTIONS {
        out.push_str("## ");
        out.push_str(name);
        out.push('\n');
        let text = section_text(rec, name);
        if !text.is_empty() {
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

fn section_text<'a>(rec: &'a TaskRecord, name: &str) -> &'a str {
    match name {
        "Context" => &rec.context,
        "Spec" => &rec.spec,
        "Success" => &rec.success,
        "Plan" => &rec.plan,
        "Log" => &rec.log,
        _ => "",
    }
}

pub fn normalize_section(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

pub fn approval_digest(rec: &TaskRecord) -> String {
    let mut h = Sha256::new();
    h.update(normalize_section(&rec.spec).as_bytes());
    h.update([0]);
    h.update(normalize_section(&rec.success).as_bytes());
    h.update([0]);
    h.update(normalize_section(&rec.plan).as_bytes());
    h.update([0]);
    for a in &rec.assertions {
        h.update(a.id.as_bytes());
        h.update([1]);
        h.update(a.text.as_bytes());
        h.update([2]);
    }
    h.update([0]);
    if let Some(m) = &rec.plan_meta {
        h.update(m.reversibility.as_deref().unwrap_or("").as_bytes());
        h.update([3]);
        for t in &m.tools {
            h.update(t.as_bytes());
            h.update([4]);
        }
        h.update([5]);
        for c in &m.checks {
            h.update(c.cmd.as_bytes());
            h.update([6]);
            for arg in &c.args {
                h.update(arg.as_bytes());
                h.update([7]);
            }
            h.update([8]);
        }
        if !m.bound_actions.is_empty() {
            h.update([9]);
            for action in &m.bound_actions {
                h.update(action.tool.as_bytes());
                h.update([10]);
                h.update(action.params.as_bytes());
                h.update([11]);
                h.update(action.destination.as_bytes());
                h.update([12]);
                h.update(action.effect.as_bytes());
                h.update([13]);
                h.update(action.grant_class.as_bytes());
                h.update([14]);
            }
        }
    }
    format!("{:x}", h.finalize())
}

pub fn registry_filename(task_id: &str, task_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(task_path.as_bytes());
    let digest = h.finalize();
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{task_id}--{hex}.yaml")
}

pub fn stamp_updated(rec: &mut TaskRecord, now: &str) {
    rec.updated_at = now.to_string();
}

pub struct RecordLock {
    _file: std::fs::File,
}

pub fn lock_path(record: &std::path::Path) -> std::path::PathBuf {
    let tasks = record.parent().unwrap_or(std::path::Path::new("."));
    let in_space = tasks.file_name().is_some_and(|n| n == "tasks")
        && tasks.parent().and_then(std::path::Path::file_name).is_some_and(|n| n == ".fabric-space");
    match tasks.parent() {
        Some(space) if in_space => space.join("tasks.lock"),
        _ => tasks.join("tasks.lock"),
    }
}

pub fn lock_folder(record: &std::path::Path) -> Result<RecordLock, FabrError> {
    let lock = lock_path(record);
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent).map_err(|e| FabrError::Io(format!("{}: {e}", parent.display())))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock)
        .map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    file.lock().map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    Ok(RecordLock { _file: file })
}

static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique_tmp(path: &std::path::Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{seq}.tmp", std::process::id()))
}

pub fn write_record_locked(
    path: &std::path::Path,
    rec: &TaskRecord,
    expected_updated_at: Option<&str>,
    _held: &RecordLock,
) -> Result<(), FabrError> {
    if let Some(expected) = expected_updated_at
        && path.exists()
    {
        let disk = parse(&std::fs::read_to_string(path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?)?;
        if disk.updated_at != expected {
            return Err(FabrError::Conflict { message: "record changed on disk".into(), node: None });
        }
    }
    let text = render(rec);
    let tmp = unique_tmp(path);
    std::fs::write(&tmp, &text).map_err(|e| FabrError::Io(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        FabrError::Io(format!("{}: {e}", path.display()))
    })
}

pub fn write_record(path: &std::path::Path, rec: &TaskRecord, expected_updated_at: Option<&str>) -> Result<(), FabrError> {
    let held = lock_folder(path)?;
    write_record_locked(path, rec, expected_updated_at, &held)
}

pub fn modify_record<T>(path: &std::path::Path, now: &str, f: impl FnOnce(&mut TaskRecord) -> T) -> Result<(TaskRecord, T), FabrError> {
    let held = lock_folder(path)?;
    let text = std::fs::read_to_string(path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
    let mut rec = parse(&text)?;
    let out = f(&mut rec);
    stamp_updated(&mut rec, now);
    write_record_locked(path, &rec, None, &held)?;
    Ok((rec, out))
}

pub fn apply_gui(rec: &mut TaskRecord, patch: &GuiPatch) {
    if let Some(r) = &patch.requests {
        rec.requests = r.clone();
    }
    if let Some(a) = &patch.approval {
        rec.approval = Some(a.clone());
    }
    if let Some(g) = &patch.grants_granted {
        rec.grants.granted = g.clone();
    }
    if let Some(ans) = &patch.answer
        && let Some(q) = rec.question.as_mut()
    {
        q.answer = Some(ans.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    fn records() -> Vec<PathBuf> {
        let mut out = Vec::new();
        for ent in walk(&fixtures_dir()) {
            if ent.extension().and_then(|e| e.to_str()) == Some("md") && ent.to_string_lossy().contains("tasks") {
                out.push(ent);
            }
        }
        out.sort();
        out
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.is_dir() {
                    out.extend(walk(&p));
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    #[test]
    fn loads_every_fixture() {
        let recs = records();
        assert_eq!(recs.len(), 12, "expected twelve fixture records, found {}", recs.len());
        for path in recs {
            let text = std::fs::read_to_string(&path).unwrap();
            let rec = parse(&text).unwrap_or_else(|e| panic!("{}: {}", path.display(), e.message()));
            assert_eq!(rec.schema, 6, "{}", path.display());
            assert_eq!(rec.pri, 0, "{}", path.display());
            assert!(render(&rec).starts_with("---\nschema: 6\n"), "{}", path.display());
            assert!(!rec.id.is_empty(), "{}", path.display());
        }
    }

    #[test]
    fn schema_two_upgrades_and_bad_values_are_refused() {
        let text =
            std::fs::read_to_string(fixtures_dir().join("01-reminder/project/.fabric-space/tasks/2026-10-24-check-printer-firmware.md"))
                .unwrap()
                .replacen("schema: 4", "schema: 2", 1)
                .replace("pri: 0\n", "")
                .replace("paused: false\nsuspended: false\nsuspend_until: null\n", "")
                .replace(", pause: false, resume: false", "");
        assert!(text.starts_with("---\nschema: 2\n"));
        let rec = parse(&text).unwrap();
        assert_eq!((rec.schema, rec.pri), (6, 0));
        assert!(
            !rec.paused
                && !rec.suspended
                && rec.suspend_until.is_none()
                && !rec.requests.pause
                && !rec.requests.resume
                && !rec.requests.complete
        );
        assert!(rec.by == By::default() && rec.needs.is_none() && rec.requests.redo.is_none());
        assert!(rec.plan_meta.as_ref().is_none_or(|m| m.requires == PlanRequires::default()));
        let out = render(&rec);
        assert!(out.contains("\nneeds: null\n") && out.contains("\nby:\n  context: null\n"));
        assert!(
            out.starts_with("---\nschema: 6\n")
                && out.contains("\npri: 0\n")
                && out.contains("\npaused: false\nsuspended: false\nsuspend_until: null\n")
        );
        let mut rec = rec;
        rec.pri = 2;
        assert_eq!(parse(&render(&rec)).unwrap().pri, 2);
        assert!(parse(&text.replacen("schema: 2", "schema: 7", 1)).is_err());
        assert!(parse(&text.replacen("schema: 2\n", "schema: 6\npri: 4\n", 1)).is_err());
    }

    #[test]
    fn due_accepts_dates_and_minutes_only() {
        for ok in ["2026-10-28", "2026-10-28T09:30", "2026-02-28T23:59"] {
            assert!(valid_due(ok), "{ok}");
        }
        for bad in ["", "tomorrow", "2026-13-01", "2026-10-28T24:00", "2026-10-28 09:30", "2026-10-28T09:30:00", "2026-10-8"] {
            assert!(!valid_due(bad), "{bad}");
        }
    }

    #[test]
    fn round_trip_preserves_fields() {
        for path in records() {
            let text = std::fs::read_to_string(&path).unwrap();
            let rec = parse(&text).unwrap();
            let again = parse(&render(&rec)).unwrap();
            assert_eq!(rec.state, again.state, "{}", path.display());
            assert_eq!(rec.id, again.id, "{}", path.display());
            assert_eq!(rec.spec, again.spec, "{}", path.display());
            assert_eq!(rec.plan, again.plan, "{}", path.display());
            assert_eq!(rec.log, again.log, "{}", path.display());
            assert_eq!(rec.assertions, again.assertions, "{}", path.display());
            assert_eq!(rec.approval, again.approval, "{}", path.display());
            assert_eq!(rec.verdict, again.verdict, "{}", path.display());
            assert_eq!(rec.grants, again.grants, "{}", path.display());
            assert_eq!(rec.question, again.question, "{}", path.display());
        }
    }

    #[test]
    fn digest_is_stable_and_sensitive() {
        let mut rec = parse(
            &std::fs::read_to_string(fixtures_dir().join("04-awaiting-approval/project/.fabric-space/tasks/2026-10-24-order-filament.md"))
                .unwrap(),
        )
        .unwrap();
        let d1 = approval_digest(&rec);
        assert_eq!(d1, approval_digest(&rec));
        rec.plan.push_str("\n7. extra step");
        assert_ne!(d1, approval_digest(&rec));
    }

    #[test]
    fn empty_bound_actions_keep_the_current_digest() {
        let mut rec = parse(
            &std::fs::read_to_string(fixtures_dir().join("04-awaiting-approval/project/.fabric-space/tasks/2026-10-24-order-filament.md"))
                .unwrap(),
        )
        .unwrap();
        assert!(rec.plan_meta.as_ref().is_some_and(|m| m.bound_actions.is_empty()));
        let previous = approval_digest(&rec);
        let text = render(&rec);
        assert!(!text.contains("bound_actions"));
        let again = parse(&text).unwrap();
        assert_eq!(previous, approval_digest(&again));
        assert_eq!(previous, approval_digest(&parse(&render(&again)).unwrap()));
        rec.plan_meta.as_mut().unwrap().bound_actions.push(BoundAction {
            tool: "bright_data_extract".into(),
            params: "{\"url\":\"https://docs.rs/ureq\"}".into(),
            destination: "https://docs.rs/ureq".into(),
            effect: "remote_read".into(),
            grant_class: "external_action".into(),
        });
        let with_bound = approval_digest(&rec);
        assert_ne!(previous, with_bound);
        assert!(render(&rec).contains("bound_actions"));
        rec.plan_meta.as_mut().unwrap().bound_actions[0].destination = "https://example.com/other".into();
        assert_ne!(with_bound, approval_digest(&rec));
        rec.plan_meta.as_mut().unwrap().bound_actions.clear();
        assert_eq!(previous, approval_digest(&rec));
        assert!(!render(&rec).contains("bound_actions"));
    }

    #[test]
    fn registry_filename_differs_by_path() {
        let a = registry_filename("t", "/home/me/3d/.fabric-space/tasks/t.md");
        let b = registry_filename("t", "/home/me/cnc/.fabric-space/tasks/t.md");
        assert_ne!(a, b);
        assert!(a.starts_with("t--") && a.ends_with(".yaml"));
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    fn sample(dir: &std::path::Path) -> (std::path::PathBuf, TaskRecord) {
        let text = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/01-reminder/project/.fabric-space/tasks/2026-10-24-check-printer-firmware.md"),
        )
        .unwrap();
        let rec = parse(&text).unwrap();
        let path = dir.join(".fabric-space/tasks").join(format!("{}.md", rec.id));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_record(&path, &rec, None).unwrap();
        (path, rec)
    }

    #[test]
    fn the_lock_file_sits_next_to_the_tasks_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = sample(dir.path());
        assert_eq!(lock_path(&path), dir.path().join(".fabric-space/tasks.lock"));
        assert!(lock_path(&path).exists());
    }

    #[test]
    fn concurrent_read_modify_write_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = sample(dir.path());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        modify_record(&path, "2026-10-25T00:00:00Z", |r| r.cost_usd += 1.0).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let rec = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(rec.cost_usd, 160.0);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn temp_names_are_unique_per_write() {
        let p = std::path::Path::new("/x/.fabric-space/tasks/a.md");
        assert_ne!(unique_tmp(p), unique_tmp(p));
    }
}
