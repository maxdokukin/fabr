use crate::board::{Activity, Board, Extras};
use crate::caps::{Harness, executor_rules, requires_of, valid_rule};
use crate::claude::{denial_text, is_session_id, rule_for, tool_line};
use crate::engine::{
    DefineInput, DraftAssertion, End, Engine, EvaluateInput, ExecStatus, ExecuteInput, Judgement, PlanInput, PlanOut, Run, ToolUse, Watch,
};
use crate::link::{list_records, load_record, update_record};
use crate::now_utc;
use crate::state::{append_log, clean_message, is_terminal, pending_tail, question_open, stage_label, state_name, suspended_now};
use crate::store::mutate;
use crate::verify;
use fabr_core::FabrError;
use fabr_core::taskrec::{
    Approval, Assertion, Author, Check, PlanMeta, Question, RedoStage, Sessions, State, Step, TaskRecord, Verdict, VerdictItem,
    approval_digest, registry_filename,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_STAGE_ERRORS: u32 = 3;
pub const MAX_SAME_DENIALS: usize = 3;
pub const PLAN_SECTIONS: [&str; 3] = ["### Inputs", "### Outputs", "### Steps"];
const GATHER_FILE_CAP: usize = 8 * 1024;
const GATHER_TOTAL_CAP: usize = 32 * 1024;
const GATHER_LIST_CAP: usize = 200;

pub struct Ctl {
    pub board: Arc<Board>,
    pub check_timeout_s: u64,
    pub harness: Harness,
    pub root: Option<PathBuf>,
    pub max_task_usd: f64,
}

impl Ctl {
    pub fn new(space: &Path, heartbeat_interval_s: u64) -> Ctl {
        Ctl::with_board(Board::new(space, heartbeat_interval_s, None), verify::DEFAULT_CHECK_TIMEOUT_S)
    }

    pub fn with_board(board: Arc<Board>, check_timeout_s: u64) -> Ctl {
        Ctl { board, check_timeout_s, harness: Harness::standard(), root: None, max_task_usd: 0.0 }
    }
}

pub fn approval_allows_execute(rec: &TaskRecord) -> Result<(), FabrError> {
    match check_approval(rec)? {
        None => Ok(()),
        Some(reason) => Err(FabrError::BadRequest(reason.into())),
    }
}

fn check_approval(rec: &TaskRecord) -> Result<Option<&'static str>, FabrError> {
    let Some(approval) = &rec.approval else {
        return Err(FabrError::BadRequest("approval is missing".into()));
    };
    if approval.status != "approved" {
        return Err(FabrError::BadRequest(format!("approval is {}", approval.status)));
    }
    if approval.by.as_deref() == Some("auto") && rec.plan_meta.as_ref().and_then(|m| m.reversibility.as_deref()) == Some("irreversible") {
        return Ok(Some("auto approval is not allowed for an irreversible plan"));
    }
    if let Some(meta) = &rec.plan_meta {
        validate_plan(meta)?;
    }
    if approval.digest.as_deref() != Some(approval_digest(rec).as_str()) {
        return Ok(Some("approval digest mismatch"));
    }
    Ok(None)
}

fn approval_problem(rec: &TaskRecord) -> Option<String> {
    match check_approval(rec) {
        Ok(None) => None,
        Ok(Some(reason)) => Some(reason.to_string()),
        Err(e) => Some(e.message()),
    }
}

pub fn validate_plan(meta: &PlanMeta) -> Result<(), FabrError> {
    for tool in &meta.tools {
        if !valid_rule(tool) {
            return Err(FabrError::BadRequest(format!("unsupported tool {tool}")));
        }
    }
    for check in &meta.checks {
        verify::validate_check(check)?;
    }
    Ok(())
}

pub fn missing_sections(plan: &str) -> Vec<&'static str> {
    PLAN_SECTIONS.iter().filter(|h| !plan.lines().any(|l| l.trim_end().eq_ignore_ascii_case(h))).copied().collect()
}

pub fn task_path_key(path: &Path) -> String {
    let canon = path.canonicalize().unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    canon.to_string_lossy().trim_end_matches('/').to_string()
}

pub fn registry_path(fabric_space: &Path, task_id: &str, task_path: &str) -> PathBuf {
    fabric_space.join("registry").join(registry_filename(task_id, task_path))
}

pub fn remove_registry(fabric_space: &Path, task_id: &str, task_path: &str) {
    let path = registry_path(fabric_space, task_id, task_path);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("remove registry {}: {e}", path.display()),
    }
}

pub fn write_registry(
    fabric_space: &Path,
    node_dir: &Path,
    record_path: &Path,
    rec: &TaskRecord,
    heartbeat_interval_s: u64,
) -> Result<(), FabrError> {
    Board::new(fabric_space, heartbeat_interval_s, None).publish(node_dir, record_path, rec, Extras::default())
}

fn mark(ctl: &Ctl, node_dir: &Path, record_path: &Path, rec: &TaskRecord, activity: Activity) {
    if let Err(e) = ctl.board.publish(node_dir, record_path, rec, Extras::doing(activity)) {
        log::warn!("registry write failed for {}: {}", rec.id, e.message());
    }
}

fn forget(ctl: &Ctl, rec: &TaskRecord, record_path: &Path) {
    ctl.board.remove(&rec.id, &task_path_key(record_path));
}

pub struct ErrorLine {
    pub at: String,
    pub label: String,
    pub count: u32,
    pub message: String,
}

pub fn parse_error_line(line: &str) -> Option<ErrorLine> {
    let line = line.strip_prefix("- ").unwrap_or(line);
    let (at, rest) = line.split_once(' ')?;
    let (label, tail) = rest.split_once(": ")?;
    let tail = tail.strip_prefix("error ")?;
    let (count, message) = tail.split_once(": ")?;
    Some(ErrorLine { at: at.to_string(), label: label.to_string(), count: count.parse().ok()?, message: message.to_string() })
}

fn is_tool_line(msg: &str) -> bool {
    msg.chars().next().is_some_and(|c| c.is_ascii_uppercase()) || msg.starts_with("asked ") || msg.starts_with("denied ")
}

pub fn trailing_error(log: &str, label: &str) -> Option<ErrorLine> {
    for line in log.lines().rev().filter(|l| !l.trim().is_empty()) {
        if let Some(err) = parse_error_line(line) {
            return (err.label == label).then_some(err);
        }
        let body = line.strip_prefix("- ").unwrap_or(line);
        let is_tool = body
            .split_once(' ')
            .and_then(|(_, rest)| rest.strip_prefix(label))
            .and_then(|rest| rest.strip_prefix(": "))
            .is_some_and(is_tool_line);
        if !is_tool {
            return None;
        }
    }
    None
}

fn stage_error(record_path: &Path, ctl: &Ctl, err: FabrError) -> Result<(), FabrError> {
    let message = clean_message(&err.message(), 300);
    let outcome = mutate(record_path, |r| {
        let label = stage_label(r);
        let count = trailing_error(&r.log, label).map_or(0, |e| e.count) + 1;
        append_log(&mut r.log, label, &format!("error {count}: {message}"));
        let fatal = message.starts_with(crate::claude::SPEND_CAP);
        if count < MAX_STAGE_ERRORS && !fatal {
            return None;
        }
        r.state = State::Failed;
        r.step = None;
        r.waiting = false;
        r.verdict = None;
        r.requests.delegate = false;
        let why = if fatal { "stopped".to_string() } else { format!("failed after {count} errors") };
        append_log(&mut r.log, label, &format!("{why}: {message}"));
        Some(())
    });
    match outcome {
        Ok((rec, Some(()))) => {
            forget(ctl, &rec, record_path);
            Ok(())
        }
        Ok((_, None)) => Err(err),
        Err(write_err) => {
            log::warn!("could not log the stage error on {}: {}", record_path.display(), write_err.message());
            Err(err)
        }
    }
}

pub fn advance(
    node_dir: &Path,
    record_path: &Path,
    fabric_space: &Path,
    heartbeat_interval_s: u64,
    engine: &impl Engine,
) -> Result<(), FabrError> {
    let ctl = Ctl::new(fabric_space, heartbeat_interval_s);
    advance_with(node_dir, record_path, &ctl, engine)
}

pub fn advance_with(node_dir: &Path, record_path: &Path, ctl: &Ctl, engine: &impl Engine) -> Result<(), FabrError> {
    let rec = load_record(record_path)?;
    match step_once(node_dir, record_path, ctl, engine, rec) {
        Ok(()) => Ok(()),
        Err(err) => stage_error(record_path, ctl, err),
    }
}

fn step_once(node_dir: &Path, record_path: &Path, ctl: &Ctl, engine: &impl Engine, mut rec: TaskRecord) -> Result<(), FabrError> {
    let mut base = rec.clone();
    if is_terminal(&rec) && !pending_tail(&rec) {
        forget(ctl, &rec, record_path);
        return Ok(());
    }
    if rec.paused || suspended_now(&rec) {
        return Ok(());
    }
    if rec.requests.redelegate {
        let state = state_name(rec.state);
        append_log(&mut rec.log, "notify", &format!("ignored request redelegate in state {state}"));
        rec.requests.redelegate = false;
        return persist(record_path, &mut rec, &mut base);
    }
    if let Some(stage) = rec.requests.redo {
        if !matches!(rec.state, State::Task | State::Defined) {
            return ignore_request(record_path, &mut rec, &mut base, "redo");
        }
        apply_redo(&mut rec, stage);
        return persist(record_path, &mut rec, &mut base);
    }
    let cx = Cx { node_dir, record_path, ctl };
    match rec.state {
        State::Reminder => {
            if rec.requests.cancel {
                return ignore_request(record_path, &mut rec, &mut base, "cancel");
            }
            if !rec.requests.delegate {
                return Ok(());
            }
            rec.requests.delegate = false;
            rec.state = State::Task;
            rec.step = Some(Step::Gather);
            rec.waiting = false;
            append_log(&mut rec.log, "task", "delegated");
            persist(record_path, &mut rec, &mut base)
        }
        State::Task => match rec.step {
            Some(Step::Define) => define_stage(&cx, &mut rec, &mut base, engine),
            Some(Step::Plan) => plan_stage(&cx, &mut rec, &mut base, engine, false),
            _ => gather_stage(&cx, &mut rec, &mut base),
        },
        State::Defined => {
            if rec.requests.cancel {
                return ignore_request(record_path, &mut rec, &mut base, "cancel");
            }
            let status = rec.approval.as_ref().map(|a| a.status.as_str()).unwrap_or("");
            if status == "refused" {
                let note = rec.approval.as_ref().and_then(|a| a.note.clone());
                match &note {
                    Some(n) => append_log(&mut rec.log, "approve", &format!("refused: {n}")),
                    None => append_log(&mut rec.log, "approve", "refused"),
                }
                return plan_stage(&cx, &mut rec, &mut base, engine, true);
            }
            if status == "pending" && auto_approvable(ctl, &rec) {
                let digest = approval_digest(&rec);
                rec.approval = Some(Approval {
                    status: "approved".into(),
                    by: Some("auto".into()),
                    at: Some(now_utc()),
                    note: None,
                    digest: Some(digest),
                });
                rec.waiting = false;
                append_log(&mut rec.log, "approve", "approved by auto");
                return persist(record_path, &mut rec, &mut base);
            }
            if status != "approved" {
                return Ok(());
            }
            if !harness_gate(&cx, &mut rec, &mut base, "execute")? {
                return Ok(());
            }
            if let Some(reason) = check_approval(&rec)? {
                let digest = approval_digest(&rec);
                rec.approval = Some(Approval { status: "pending".into(), by: None, at: None, note: None, digest: Some(digest) });
                rec.waiting = true;
                append_log(&mut rec.log, "approve", &format!("{reason}, reset to pending"));
                return persist(record_path, &mut rec, &mut base);
            }
            if other_running(node_dir, record_path)? {
                if let Err(e) = ctl.board.publish(node_dir, record_path, &rec, Extras::doing(Activity::Queued)) {
                    log::warn!("registry write failed for {}: {}", rec.id, e.message());
                }
                log::info!("node already has an in_progress task");
                return Ok(());
            }
            rec.state = State::InProgress;
            rec.step = Some(Step::Execute);
            rec.waiting = false;
            rec.sessions.execute = None;
            rec.sessions.evaluate = None;
            append_log(&mut rec.log, "execute", "started");
            persist(record_path, &mut rec, &mut base)?;
            mark(ctl, node_dir, record_path, &rec, Activity::Tool);
            Ok(())
        }
        State::InProgress => {
            if !matches!(rec.step, Some(Step::Learn | Step::Notify)) {
                let label = stage_label(&rec);
                if !harness_gate(&cx, &mut rec, &mut base, label)? {
                    return Ok(());
                }
            }
            match rec.step {
                Some(Step::Evaluate) => evaluate_stage(&cx, &mut rec, &mut base, engine),
                Some(Step::Learn | Step::Notify) => tail_step(&cx, &mut rec, &mut base),
                _ => execute_stage(&cx, &mut rec, &mut base, engine),
            }
        }
        State::Succeeded | State::Failed => match rec.step {
            Some(Step::Learn | Step::Notify) => tail_step(&cx, &mut rec, &mut base),
            _ => {
                forget(ctl, &rec, record_path);
                Ok(())
            }
        },
    }
}

struct Cx<'a> {
    node_dir: &'a Path,
    record_path: &'a Path,
    ctl: &'a Ctl,
}

impl Cx<'_> {
    fn mark(&self, rec: &TaskRecord, activity: Activity) {
        mark(self.ctl, self.node_dir, self.record_path, rec, activity);
    }

    fn blocked(&self, rec: &mut TaskRecord, base: &mut TaskRecord, label: &str, resume: Option<&str>) -> Result<bool, FabrError> {
        let cap = self.ctl.max_task_usd;
        if cap > 0.0 && rec.cost_usd >= cap {
            rec.state = State::Failed;
            rec.step = None;
            rec.waiting = false;
            rec.verdict = None;
            rec.requests.delegate = false;
            append_log(&mut rec.log, label, &format!("stopped: task spend cap reached (${:.2} of ${cap:.2})", rec.cost_usd));
            persist(self.record_path, rec, base)?;
            forget(self.ctl, rec, self.record_path);
            return Ok(true);
        }
        if !resume.is_some_and(|id| chat_running(self.node_dir, id)) {
            return Ok(false);
        }
        let note = "waiting for a chat on this session to finish";
        if log_message(rec.log.lines().last().unwrap_or(""), label) != Some(note) {
            append_log(&mut rec.log, label, note);
            persist(self.record_path, rec, base)?;
        }
        self.mark(rec, Activity::Queued);
        Ok(true)
    }
}

pub fn chat_running(node_dir: &Path, session: &str) -> bool {
    if !is_session_id(session) {
        return false;
    }
    let lock = node_dir.join(".fabric-space").join("chat").join(format!("{session}.lock"));
    let Ok(text) = std::fs::read_to_string(&lock) else { return false };
    let pid = text.split_whitespace().next().and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
    if crate::pgroup::alive(pid) {
        return true;
    }
    let _ = std::fs::remove_file(&lock);
    false
}

fn session_of(id: &Option<String>) -> Option<String> {
    id.clone().filter(|s| is_session_id(s))
}

#[derive(Debug, Clone, Copy)]
enum Slot {
    Plan,
    Execute,
    Evaluate,
}

fn slot_mut(sessions: &mut Sessions, slot: Slot) -> &mut Option<String> {
    match slot {
        Slot::Plan => &mut sessions.plan,
        Slot::Execute => &mut sessions.execute,
        Slot::Evaluate => &mut sessions.evaluate,
    }
}

fn halt_requested(rec: &TaskRecord) -> bool {
    (rec.requests.cancel && rec.state == State::InProgress) || must_stop(rec)
}

struct Live<'a, 'b> {
    cx: &'a Cx<'b>,
    rec: &'a mut TaskRecord,
    base: &'a mut TaskRecord,
    slot: Slot,
    label: &'static str,
    log_tools: bool,
}

impl Live<'_, '_> {
    fn save(&mut self) {
        if let Err(e) = persist(self.cx.record_path, self.rec, self.base) {
            log::warn!("could not save progress on {}: {}", self.cx.record_path.display(), e.message());
        }
    }
}

impl Watch for Live<'_, '_> {
    fn session(&mut self, id: &str) {
        let field = slot_mut(&mut self.rec.sessions, self.slot);
        if field.as_deref() != Some(id) {
            *field = Some(id.to_string());
            self.save();
        }
    }

    fn tool(&mut self, tool: &ToolUse) {
        if self.log_tools {
            append_log(&mut self.rec.log, self.label, &clean_message(&tool_line(self.cx.node_dir, tool), 160));
            self.save();
        }
        self.cx.mark(self.rec, Activity::Tool);
    }

    fn stop(&mut self) -> bool {
        crate::runner::stopping() || load_record(self.cx.record_path).is_ok_and(|r| halt_requested(&r))
    }
}

fn settle<T>(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, slot: Slot, step: &str, run: Run<T>) -> Result<Option<T>, FabrError> {
    if let Some(id) = &run.session_id {
        *slot_mut(&mut rec.sessions, slot) = Some(id.clone());
    }
    if run.cost_usd > 0.0 {
        rec.cost_usd = ((rec.cost_usd + run.cost_usd) * 1e6).round() / 1e6;
    }
    cx.ctl.board.spent(&rec.id, step, run.session_id.as_deref(), &run.usage, run.cost_usd);
    match run.end {
        End::Done(out) => Ok(Some(out)),
        End::Stopped => {
            append_log(&mut rec.log, step, "run stopped");
            persist(cx.record_path, rec, base)?;
            Ok(None)
        }
        End::Failed(err) => {
            persist(cx.record_path, rec, base)?;
            Err(err)
        }
    }
}

fn auto_approvable(ctl: &Ctl, rec: &TaskRecord) -> bool {
    ctl.harness.auto_mode
        && rec.needs.is_none()
        && rec.plan_meta.as_ref().is_some_and(|m| m.reversibility.as_deref() != Some("irreversible") && validate_plan(m).is_ok())
}

fn harness_gate(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, label: &str) -> Result<bool, FabrError> {
    let Some(meta) = rec.plan_meta.clone() else { return Ok(true) };
    match cx.ctl.harness.missing(&meta) {
        None => {
            if rec.needs.is_some() {
                rec.needs = None;
                persist(cx.record_path, rec, base)?;
            }
            Ok(true)
        }
        Some(missing) => {
            let needs = format!("harness: {missing}");
            if rec.needs.as_deref() != Some(needs.as_str()) {
                rec.needs = Some(needs);
                append_log(&mut rec.log, label, &format!("needs harness: {missing}"));
                persist(cx.record_path, rec, base)?;
            }
            cx.mark(rec, Activity::Parked);
            Ok(false)
        }
    }
}

pub fn apply_redo(rec: &mut TaskRecord, stage: RedoStage) {
    rec.requests.redo = None;
    match stage {
        RedoStage::Define => {
            if rec.by.spec == Some(Author::Agent) {
                rec.spec.clear();
                rec.by.spec = None;
            }
            if rec.by.success == Some(Author::Agent) {
                rec.success.clear();
                rec.by.success = None;
            }
            rec.assertions.clear();
            rec.sessions.plan = None;
            rec.step = Some(Step::Define);
            rec.waiting = question_open(rec);
            append_log(&mut rec.log, "redo", "define by user");
        }
        RedoStage::Plan => {
            rec.step = Some(Step::Plan);
            rec.waiting = false;
            append_log(&mut rec.log, "redo", "plan by user");
        }
    }
    rec.state = State::Task;
    rec.plan.clear();
    rec.by.plan = None;
    rec.plan_meta = None;
    rec.approval = None;
    rec.needs = None;
}

fn tail_step(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    if rec.step == Some(Step::Learn) {
        rec.step = Some(Step::Notify);
        let msg = match fabr_core::knowledge::capture(cx.node_dir, cx.record_path, rec, &now_utc()) {
            Ok(0) => "nothing new".to_string(),
            Ok(n) => format!("captured {n} answer{}", if n == 1 { "" } else { "s" }),
            Err(e) => format!("capture failed: {}", clean_message(&e.message(), 160)),
        };
        append_log(&mut rec.log, "learn", &msg);
        persist(cx.record_path, rec, base)?;
        cx.mark(rec, Activity::Tool);
        return Ok(());
    }
    let result = state_name(rec.state);
    append_log(&mut rec.log, "notify", &format!("result {result}"));
    rec.step = None;
    persist(cx.record_path, rec, base)?;
    forget(cx.ctl, rec, cx.record_path);
    Ok(())
}

fn gather_stage(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    let mut view = rec.clone();
    view.step = Some(Step::Gather);
    cx.mark(&view, Activity::Tool);
    if rec.context.trim().is_empty() {
        rec.by.context = Some(Author::Agent);
    }
    append_gathered(&mut rec.context, &gather_bundle(cx.node_dir, cx.ctl.root.as_deref()));
    rec.step = Some(Step::Define);
    rec.waiting = false;
    append_log(&mut rec.log, "gather", "context bundle written");
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Tool);
    Ok(())
}

pub fn folder_chain(node_dir: &Path, root: Option<&Path>) -> Vec<PathBuf> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let node = canon(node_dir);
    let Some(root) = root.map(canon).filter(|r| node.starts_with(r)) else { return vec![node] };
    let mut out = Vec::new();
    let mut dir = Some(node.as_path());
    while let Some(d) = dir {
        out.push(d.to_path_buf());
        if d == root {
            break;
        }
        dir = d.parent();
    }
    out
}

pub fn demote(text: &str, level: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes < level && line[hashes..].starts_with(' ') {
            out.push_str(&"#".repeat(level));
            out.push_str(&line[hashes..]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out.trim_end_matches('\n').to_string()
}

fn capped_text(path: &Path, cap: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(cap)]).trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(if bytes.len() > cap { format!("{text}\n(cut at {cap} bytes)") } else { text })
}

pub fn gather_bundle(node_dir: &Path, root: Option<&Path>) -> String {
    let chain = folder_chain(node_dir, root);
    let label = |dir: &Path, i: usize| {
        let name = match root.and_then(|r| r.canonicalize().ok()).and_then(|r| dir.strip_prefix(r).ok().map(Path::to_path_buf)) {
            Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
            Some(rel) => rel.to_string_lossy().into_owned(),
            None => dir.to_string_lossy().into_owned(),
        };
        if i == 0 { format!("{name} (this folder)") } else { name }
    };
    let mut out = String::from("### Gathered\n");
    let mut used = 0usize;
    let mut cut = false;
    for (file, heading) in [("rules.md", "#### Rules"), ("knowledge.md", "#### Knowledge")] {
        let mut part = String::new();
        for (i, dir) in chain.iter().enumerate() {
            let Some(text) = capped_text(&dir.join(".fabric-space").join(".agents").join(file), GATHER_FILE_CAP) else { continue };
            let block = format!("From {}:\n{}\n\n", label(dir, i), demote(&text, 5));
            if used + block.len() > GATHER_TOTAL_CAP {
                cut = true;
                continue;
            }
            used += block.len();
            part.push_str(&block);
        }
        if !part.is_empty() {
            out.push_str(&format!("{heading}\n{part}"));
        }
    }
    let mut learned = String::new();
    for (i, dir) in chain.iter().enumerate() {
        for r in fabr_core::knowledge::read_learned(dir).unwrap_or_default() {
            let when = if r.applies_when.is_empty() { String::new() } else { format!("when {}: ", demote(&r.applies_when, 5)) };
            let line = format!("- {when}{} (learned in {}, confirmed {}x)\n", demote(&r.text, 5), label(dir, i), r.count);
            if used + line.len() > GATHER_TOTAL_CAP {
                cut = true;
                continue;
            }
            used += line.len();
            learned.push_str(&line);
        }
    }
    if !learned.is_empty() {
        out.push_str(&format!("#### Learned from earlier answers\n{learned}\n"));
    }
    if cut {
        out.push_str("(more rules and knowledge were left out)\n\n");
    }
    out.push_str("#### Files\n");
    let mut lines = Vec::new();
    if let Ok(rd) = std::fs::read_dir(node_dir) {
        let mut names: Vec<_> = rd.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for ent in names {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if lines.len() >= GATHER_LIST_CAP {
                lines.push("(more files)".to_string());
                break;
            }
            let path = ent.path();
            if path.is_dir() {
                lines.push(format!("{name}/"));
            } else if ent.metadata().is_ok_and(|m| m.len() <= 4096) {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let preview: String = text.lines().next().unwrap_or("").trim().trim_start_matches('#').trim().chars().take(120).collect();
                lines.push(if preview.is_empty() { name } else { format!("{name}: {}", demote(&preview, 5)) });
            } else {
                lines.push(name);
            }
        }
    }
    out.push_str(&if lines.is_empty() { "(empty)".to_string() } else { lines.join("\n") });
    out.push('\n');
    out
}

fn append_gathered(context: &mut String, bundle: &str) {
    if let Some(i) = context.find("### Gathered") {
        context.truncate(i);
    }
    if !context.is_empty() && !context.ends_with('\n') {
        context.push('\n');
    }
    context.push_str(bundle);
}

pub fn answers_text(rec: &TaskRecord) -> String {
    let mut out = String::new();
    let answered = rec.answers.iter().chain(rec.question.iter()).filter(|q| q.answer.is_some());
    for q in answered {
        out.push_str(&format!("\n\nAlready asked: {}\nUser answered: {}", q.text, reply_text(q)));
    }
    out
}

fn reply_text(q: &Question) -> String {
    let Some(a) = q.answer.as_ref() else { return String::new() };
    let mut parts = Vec::new();
    if let Some(value) = a.value.as_deref().filter(|v| !v.is_empty()) {
        parts.push(value.to_string());
    }
    if let Some(replacement) = a.replacement.as_deref().filter(|v| !v.is_empty()) {
        parts.push(format!("use instead: {replacement}"));
    }
    if let Some(granted) = a.granted.as_ref().filter(|g| !g.is_empty()) {
        let fields: Vec<String> = granted.iter().map(|g| format!("{} ({})", g.field, g.scope)).collect();
        parts.push(format!("granted {}", fields.join(", ")));
    }
    let mut reply = parts.join("; ");
    if q.kind == "choice"
        && let Some(prior) = &q.prior
    {
        reply.push_str(&format!(" (earlier choice: {})", prior.text));
    }
    reply
}

pub fn context_with_answers(rec: &TaskRecord) -> String {
    let mut out = rec.context.clone();
    out.push_str(&answers_text(rec));
    out
}

fn next_question_id(rec: &TaskRecord) -> String {
    let highest = rec
        .answers
        .iter()
        .chain(rec.question.iter())
        .filter_map(|q| q.id.strip_prefix('q').and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    format!("q{}", highest + 1)
}

fn ask(rec: &mut TaskRecord, label: &str, text: String) {
    let id = next_question_id(rec);
    rec.question = Some(Question {
        id: id.clone(),
        kind: "open".into(),
        text,
        options: None,
        prior: None,
        fields: None,
        asked_at: Some(now_utc()),
        answer: None,
    });
    rec.waiting = true;
    append_log(&mut rec.log, label, &format!("asked {id}"));
}

fn fold_question(rec: &mut TaskRecord) {
    if rec.question.as_ref().is_some_and(|q| q.answer.is_some())
        && let Some(q) = rec.question.take()
    {
        rec.answers.push(q);
        rec.waiting = false;
    }
}

fn define_stage(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, engine: &impl Engine) -> Result<(), FabrError> {
    if rec.waiting && question_open(rec) {
        return Ok(());
    }
    fold_question(rec);
    let spec_human = rec.by.spec == Some(Author::Human);
    let success_human = rec.by.success == Some(Author::Human);
    if spec_human && success_human && !rec.assertions.is_empty() {
        rec.question = None;
        rec.waiting = false;
        rec.step = Some(Step::Plan);
        append_log(&mut rec.log, "define", "spec kept");
        persist(cx.record_path, rec, base)?;
        cx.mark(rec, Activity::Model);
        return Ok(());
    }
    let (title, context, answers, resume) = (rec.title.clone(), rec.context.clone(), answers_text(rec), session_of(&rec.sessions.plan));
    if cx.blocked(rec, base, "define", resume.as_deref())? {
        return Ok(());
    }
    cx.mark(rec, Activity::Model);
    let input = DefineInput { cwd: cx.node_dir, resume: resume.as_deref(), title: &title, context: &context, answers: &answers };
    let run = engine.define(&input, &mut Live { cx, rec, base, slot: Slot::Plan, label: "define", log_tools: false });
    let Some(draft) = settle(cx, rec, base, Slot::Plan, "define", run)? else { return Ok(()) };
    if let Some(text) = draft.question.filter(|q| !q.trim().is_empty()) {
        rec.step = Some(Step::Define);
        ask(rec, "define", text);
        persist(cx.record_path, rec, base)?;
        cx.mark(rec, Activity::Parked);
        return Ok(());
    }
    let keep_assertions = (spec_human || success_human) && !rec.assertions.is_empty();
    if (!spec_human && draft.spec.trim().is_empty())
        || (!success_human && draft.success.trim().is_empty())
        || (!keep_assertions && draft.assertions.is_empty())
    {
        return Err(FabrError::BadRequest(if draft.assertions.is_empty() && !keep_assertions {
            "define returned no assertions; 1 to 3 are required".into()
        } else {
            "spec is incomplete".into()
        }));
    }
    if !spec_human {
        rec.spec = demote(draft.spec.trim(), 3);
        rec.by.spec = Some(Author::Agent);
    }
    if !success_human {
        rec.success = demote(draft.success.trim(), 3);
        rec.by.success = Some(Author::Agent);
    }
    if !keep_assertions {
        check_assertion_count(&draft.assertions)?;
        rec.assertions = draft.assertions.into_iter().map(|a| Assertion { id: a.id, text: a.text }).collect();
    }
    let mut requested: Vec<String> = Vec::new();
    for field in draft.grants.into_iter().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()) {
        if !requested.contains(&field) {
            requested.push(field);
        }
    }
    rec.grants.requested = requested;
    rec.question = None;
    rec.waiting = false;
    rec.step = Some(Step::Plan);
    append_log(&mut rec.log, "define", "spec written");
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Model);
    Ok(())
}

fn run_plan(
    cx: &Cx,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    engine: &impl Engine,
    note: Option<&str>,
    rejection: Option<&str>,
) -> Result<Option<PlanOut>, FabrError> {
    let allowed = cx.ctl.harness.planning();
    let r = rec.clone();
    let resume = session_of(&r.sessions.plan);
    if cx.blocked(rec, base, "plan", resume.as_deref())? {
        return Ok(None);
    }
    let answers = answers_text(&r);
    let input = PlanInput {
        cwd: cx.node_dir,
        resume: resume.as_deref(),
        title: &r.title,
        context: &r.context,
        spec: &r.spec,
        success: &r.success,
        assertions: &r.assertions,
        answers: &answers,
        refusal_note: note,
        rejection,
        allowed_tools: &allowed.tools,
        allowed_checks: &allowed.checks,
    };
    let run = engine.plan(&input, &mut Live { cx, rec, base, slot: Slot::Plan, label: "plan", log_tools: false });
    settle(cx, rec, base, Slot::Plan, "plan", run)
}

fn plan_problem(draft: &PlanOut, meta: &PlanMeta, plan_kept: bool) -> Option<String> {
    let mut problems = Vec::new();
    let missing = missing_sections(&draft.plan);
    if !plan_kept && !missing.is_empty() {
        problems.push(format!("the plan is missing {}", missing.join(", ")));
    }
    if let Err(e) = validate_plan(meta) {
        problems.push(e.message());
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

fn plan_stage(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, engine: &impl Engine, refused: bool) -> Result<(), FabrError> {
    let note = if refused { Some(rec.approval.as_ref().and_then(|a| a.note.clone()).unwrap_or_default()) } else { None };
    let mut view = rec.clone();
    view.step = Some(Step::Plan);
    cx.mark(&view, Activity::Model);
    let human_plan = rec.by.plan == Some(Author::Human) && !rec.plan.trim().is_empty();
    let kept = if human_plan { rec.plan_meta.clone().filter(|m| !m.checks.is_empty()) } else { None };
    let (mut meta, draft) = match kept {
        Some(meta) => (meta, None),
        None => {
            let Some(mut draft) = run_plan(cx, rec, base, engine, note.as_deref(), None)? else { return Ok(()) };
            let mut meta = meta_of(&draft, human_plan)?;
            ground_checks(cx.node_dir, &mut meta, &draft.plan, &mut rec.log);
            if let Some(problem) = plan_problem(&draft, &meta, human_plan) {
                let allowed = cx.ctl.harness.planning();
                let rejection =
                    format!("{problem}; allowed tools: {}; allowed checks: {}", allowed.tools.join(", "), allowed.checks.join(", "));
                append_log(&mut rec.log, "plan", &format!("retry after rejection: {}", clean_message(&problem, 200)));
                let Some(again) = run_plan(cx, rec, base, engine, note.as_deref(), Some(&rejection))? else { return Ok(()) };
                draft = again;
                meta = meta_of(&draft, human_plan)?;
                ground_checks(cx.node_dir, &mut meta, &draft.plan, &mut rec.log);
            }
            let missing = missing_sections(&draft.plan);
            if !human_plan && !missing.is_empty() {
                persist(cx.record_path, rec, base)?;
                return Err(FabrError::BadRequest(format!("the plan is missing {}", missing.join(", "))));
            }
            (meta, Some(draft))
        }
    };
    meta.requires = requires_of(&meta);
    let invalid = validate_plan(&meta).err();
    if let Some(draft) = draft
        && !human_plan
    {
        rec.plan = demote(draft.plan.trim(), 3);
        rec.by.plan = Some(Author::Agent);
    }
    rec.needs = None;
    if let Some(err) = invalid {
        let missing = cx.ctl.harness.missing(&meta).unwrap_or_else(|| clean_message(&err.message(), 160));
        rec.needs = Some(format!("harness: {missing}"));
        append_log(&mut rec.log, "plan", &format!("needs harness: {missing}"));
    }
    rec.plan_meta = Some(meta);
    let digest = approval_digest(rec);
    rec.approval = Some(Approval { status: "pending".into(), by: None, at: None, note: None, digest: Some(digest) });
    rec.state = State::Defined;
    rec.step = Some(Step::Approve);
    rec.waiting = true;
    append_log(&mut rec.log, "plan", if refused { "redrafted" } else { "plan written" });
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Model);
    Ok(())
}

fn check_assertion_count(list: &[DraftAssertion]) -> Result<(), FabrError> {
    match list.len() {
        0..=3 => Ok(()),
        n => Err(FabrError::BadRequest(format!("define returned {n} assertions; 1 to 3 are allowed"))),
    }
}

fn meta_of(draft: &PlanOut, plan_kept: bool) -> Result<PlanMeta, FabrError> {
    if !plan_kept && draft.plan.trim().is_empty() {
        return Err(FabrError::BadRequest("plan is incomplete".into()));
    }
    let reversibility = if draft.reversibility.trim() == "irreversible" { "irreversible" } else { "reversible" };
    let mut tools: Vec<String> = Vec::new();
    for t in draft.tools.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        if !tools.contains(&t) {
            tools.push(t);
        }
    }
    Ok(PlanMeta {
        reversibility: Some(reversibility.into()),
        tools,
        assumptions: Vec::new(),
        checks: draft.checks.iter().map(|c| Check { cmd: c.cmd.clone(), args: c.args.clone() }).collect(),
        requires: Default::default(),
    })
}

fn cancel_now(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    rec.requests.cancel = false;
    rec.state = State::Failed;
    rec.step = None;
    rec.waiting = false;
    rec.verdict = None;
    append_log(&mut rec.log, "execute", "cancelled by user");
    persist(cx.record_path, rec, base)?;
    forget(cx.ctl, rec, cx.record_path);
    Ok(())
}

fn must_stop(rec: &TaskRecord) -> bool {
    rec.requests.complete || rec.requests.pause || rec.paused || suspended_now(rec)
}

fn log_message<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let body = line.strip_prefix("- ")?;
    let (_, rest) = body.split_once(' ')?;
    rest.strip_prefix(label)?.strip_prefix(": ")
}

pub fn denied_rules(log: &str, question_id: &str) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let asked = format!("asked {question_id}");
    let Some(i) = lines.iter().position(|l| log_message(l, "execute") == Some(asked.as_str())) else { return Vec::new() };
    let mut out: Vec<String> = lines[..i]
        .iter()
        .rev()
        .map_while(|l| log_message(l, "execute").and_then(|m| m.strip_prefix("denied ")))
        .filter(|r| valid_rule(r))
        .map(str::to_string)
        .collect();
    out.reverse();
    out
}

pub fn consents(reply: &str) -> bool {
    matches!(reply.trim().to_ascii_lowercase().as_str(), "allow" | "allow." | "yes, allow" | "yes, allow.")
}

const OFFER: &str = "Reply allow to grant ";
const OFFER_END: &str = ", or say what to do instead.";

pub fn offered_rules(question: &str) -> Vec<String> {
    let Some((_, rest)) = question.split_once(OFFER) else { return Vec::new() };
    let list = rest.split(OFFER_END).next().unwrap_or("");
    list.split('`').skip(1).step_by(2).filter(|r| valid_rule(r)).map(str::to_string).collect()
}

fn apply_allow(rec: &mut TaskRecord) {
    let mut grants: Vec<(String, Vec<String>)> = Vec::new();
    for q in &rec.answers {
        if !q.answer.as_ref().and_then(|a| a.value.as_deref()).is_some_and(consents) {
            continue;
        }
        if rec.log.contains(&format!(" by user for {}; approval re-signed", q.id)) {
            continue;
        }
        let denied = denied_rules(&rec.log, &q.id);
        let rules: Vec<String> = offered_rules(&q.text)
            .into_iter()
            .filter(|r| denied.contains(r) && !rec.plan_meta.as_ref().is_some_and(|m| m.tools.contains(r)))
            .filter(|r| !grants.iter().any(|(_, g)| g.contains(r)))
            .collect();
        if !rules.is_empty() {
            grants.push((q.id.clone(), rules));
        }
    }
    if grants.is_empty() {
        return;
    }
    let meta = rec.plan_meta.get_or_insert_with(|| PlanMeta {
        reversibility: Some("reversible".into()),
        tools: Vec::new(),
        assumptions: Vec::new(),
        checks: Vec::new(),
        requires: Default::default(),
    });
    for (_, rules) in &grants {
        meta.tools.extend(rules.iter().cloned());
    }
    meta.requires = requires_of(meta);
    let digest = approval_digest(rec);
    if let Some(a) = rec.approval.as_mut() {
        a.digest = Some(digest);
    }
    for (id, rules) in grants {
        append_log(&mut rec.log, "execute", &format!("allowed {} by user for {id}; approval re-signed", rules.join(", ")));
    }
}

pub fn denial_sets(log: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = log.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !log_message(line, "execute").is_some_and(|m| m.starts_with("asked q")) {
            continue;
        }
        let mut set: Vec<String> = lines[..i]
            .iter()
            .rev()
            .map_while(|l| log_message(l, "execute").and_then(|m| m.strip_prefix("denied ")))
            .map(str::to_string)
            .collect();
        set.sort();
        set.dedup();
        out.push(set);
    }
    out
}

fn unapprove(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, reason: &str) -> Result<(), FabrError> {
    let digest = approval_digest(rec);
    rec.approval = Some(Approval { status: "pending".into(), by: None, at: None, note: None, digest: Some(digest) });
    rec.state = State::Defined;
    rec.step = Some(Step::Approve);
    rec.waiting = true;
    append_log(&mut rec.log, "approve", &format!("{} before execute, reset to pending", clean_message(reason, 160)));
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Parked);
    Ok(())
}

fn execute_stage(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, engine: &impl Engine) -> Result<(), FabrError> {
    if rec.requests.cancel {
        return cancel_now(cx, rec, base);
    }
    if must_stop(rec) || (rec.waiting && question_open(rec)) {
        return Ok(());
    }
    fold_question(rec);
    if let Some(reason) = approval_problem(rec) {
        return unapprove(cx, rec, base, &reason);
    }
    apply_allow(rec);
    let resume = session_of(&rec.sessions.execute);
    if cx.blocked(rec, base, "execute", resume.as_deref())? {
        return Ok(());
    }
    let r = rec.clone();
    let allowed = executor_rules(r.plan_meta.as_ref());
    let answers = answers_text(&r);
    let input = ExecuteInput {
        cwd: cx.node_dir,
        resume: resume.as_deref(),
        title: &r.title,
        spec: &r.spec,
        plan: &r.plan,
        assertions: &r.assertions,
        answers: &answers,
        allowed: &allowed,
        read_only: cx.ctl.harness.read_only(),
    };
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Model);
    let run = engine.execute(&input, &mut Live { cx, rec, base, slot: Slot::Execute, label: "execute", log_tools: true });
    let denials = run.denials.clone();
    let Some(out) = settle(cx, rec, base, Slot::Execute, "execute", run)? else {
        if rec.requests.cancel && rec.state == State::InProgress {
            return cancel_now(cx, rec, base);
        }
        return Ok(());
    };
    if out.status == ExecStatus::Done {
        for d in &denials {
            append_log(&mut rec.log, "execute", &format!("note: was not allowed to {}", clean_message(&denial_text(cx.node_dir, d), 160)));
        }
        let summary = clean_message(&out.summary, 300);
        append_log(&mut rec.log, "execute", &format!("done: {}", if summary.is_empty() { "finished" } else { &summary }));
        rec.step = Some(Step::Evaluate);
        rec.verdict = None;
        persist(cx.record_path, rec, base)?;
        cx.mark(rec, Activity::Check);
        return Ok(());
    }
    let question = out.question.as_deref().map(str::trim).filter(|q| !q.is_empty()).map(str::to_string);
    let text = if denials.is_empty() {
        question.unwrap_or_else(|| {
            let s = out.summary.trim();
            if s.is_empty() { "The executor stopped and needs your input. What should it do?".to_string() } else { s.to_string() }
        })
    } else {
        let have = allowed;
        let (mut whats, mut lines, mut offer, mut granted) = (Vec::new(), Vec::new(), Vec::new(), false);
        for d in &denials {
            let what = denial_text(cx.node_dir, d);
            let line = match rule_for(cx.node_dir, d) {
                Some(rule) => {
                    if have.contains(&rule) {
                        granted = true;
                    } else if !offer.contains(&rule) {
                        offer.push(rule.clone());
                    }
                    format!("denied {rule}")
                }
                None => format!("denied {} (no rule fabr can grant): {what}", d.tool),
            };
            let line = clean_message(&line, 200);
            if !lines.contains(&line) {
                lines.push(line);
            }
            if !whats.contains(&what) {
                whats.push(what);
            }
        }
        let mut set: Vec<String> = lines.iter().filter_map(|l| l.strip_prefix("denied ")).map(str::to_string).collect();
        set.sort();
        let repeats = denial_sets(&rec.log).iter().rev().take_while(|s| **s == set).count();
        for line in &lines {
            append_log(&mut rec.log, "execute", line);
        }
        if repeats + 1 >= MAX_SAME_DENIALS {
            persist(cx.record_path, rec, base)?;
            return Err(FabrError::BadRequest(format!(
                "Claude Code was denied the same thing {} times in a row: {}",
                repeats + 1,
                whats.join("; ")
            )));
        }
        let mut text = format!("Claude Code was not allowed to {}.", whats.join("; "));
        if !offer.is_empty() {
            let rules: Vec<String> = offer.iter().map(|r| format!("`{r}`")).collect();
            text.push_str(&format!(" {OFFER}{}{OFFER_END}", rules.join(", ")));
        } else if granted {
            text.push_str(" It is already allowed, so the rule did not cover what it tried. Say what to do instead.");
        } else {
            text.push_str(" fabr will not grant this. Say what to do instead.");
        }
        if let Some(q) = question {
            text.push_str(&format!(" It also asks: {q}"));
        }
        text
    };
    ask(rec, "execute", text);
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Parked);
    Ok(())
}

fn evaluate_stage(cx: &Cx, rec: &mut TaskRecord, base: &mut TaskRecord, engine: &impl Engine) -> Result<(), FabrError> {
    let checks = rec.plan_meta.as_ref().map(|m| m.checks.clone()).unwrap_or_default();
    cx.mark(rec, Activity::Check);
    let outcomes = verify::run_checks(cx.node_dir, &checks, Duration::from_secs(cx.ctl.check_timeout_s))?;
    let (items, pass) = if rec.assertions.is_empty() {
        let items: Vec<VerdictItem> = outcomes
            .iter()
            .enumerate()
            .map(|(i, o)| VerdictItem { id: format!("c{}", i + 1), pass: o.pass, evidence: o.evidence.clone() })
            .collect();
        let pass = !items.is_empty() && items.iter().all(|i| i.pass);
        (items, pass)
    } else {
        if cx.blocked(rec, base, "evaluate", None)? {
            return Ok(());
        }
        cx.mark(rec, Activity::Model);
        let r = rec.clone();
        let lines: Vec<&str> = r.log.lines().filter(|l| l.contains(" execute: ")).collect();
        let log = lines[lines.len().saturating_sub(80)..].join("\n");
        let summary = r.log.lines().rev().find_map(|l| log_message(l, "execute").and_then(|m| m.strip_prefix("done: "))).unwrap_or("");
        let input = EvaluateInput { cwd: cx.node_dir, title: &r.title, assertions: &r.assertions, summary, log: &log, checks: &outcomes };
        let run = engine.evaluate(&input, &mut Live { cx, rec, base, slot: Slot::Evaluate, label: "evaluate", log_tools: false });
        let Some(judged) = settle(cx, rec, base, Slot::Evaluate, "evaluate", run)? else { return Ok(()) };
        let items: Vec<VerdictItem> = rec
            .assertions
            .iter()
            .map(|a| match judged.items.iter().find(|i| i.id == a.id) {
                Some(i) => {
                    let evidence = clean_message(&i.evidence, 300);
                    VerdictItem {
                        id: a.id.clone(),
                        pass: i.verdict == Judgement::Pass,
                        evidence: if i.verdict == Judgement::Unchecked { format!("unchecked: {evidence}") } else { evidence },
                    }
                }
                None => VerdictItem { id: a.id.clone(), pass: false, evidence: "unchecked: no verdict returned".into() },
            })
            .collect();
        let pass = items.iter().all(|i| i.pass);
        (items, pass)
    };
    rec.verdict = Some(Verdict { result: if pass { "pass" } else { "fail" }.into(), at: now_utc(), items });
    rec.state = if pass { State::Succeeded } else { State::Failed };
    rec.step = Some(Step::Learn);
    rec.waiting = false;
    append_log(&mut rec.log, "evaluate", if pass { "verdict pass" } else { "verdict fail" });
    persist(cx.record_path, rec, base)?;
    cx.mark(rec, Activity::Tool);
    Ok(())
}

fn ground_checks(node_dir: &Path, meta: &mut PlanMeta, plan: &str, log: &mut String) {
    let created = |file: &str| {
        plan.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.contains(&file.to_ascii_lowercase())
                && ["create", "write", "add", "new", "save", "make", "generate", "append", "output"].iter().any(|v| l.contains(v))
        })
    };
    let listed_output = |file: &str| {
        let mut in_outputs = false;
        plan.lines().any(|l| {
            if l.starts_with('#') {
                in_outputs = l.trim().eq_ignore_ascii_case("### Outputs");
                return false;
            }
            in_outputs && l.contains(file)
        })
    };
    meta.checks.retain(|c| {
        let files = verify::file_args(c);
        let unknown = !files.is_empty() && files.iter().all(|f| !node_dir.join(f).exists() && !created(f) && !listed_output(f));
        if unknown {
            append_log(log, "plan", &format!("dropped check {} {}: unknown file", c.cmd, c.args.join(" ")));
        }
        !unknown
    });
}

fn other_running(node_dir: &Path, current: &Path) -> Result<bool, FabrError> {
    let current = task_path_key(current);
    Ok(list_records(node_dir)?
        .into_iter()
        .any(|(path, rec)| rec.state == State::InProgress && !rec.paused && !suspended_now(&rec) && task_path_key(&path) != current))
}

fn ignore_request(path: &Path, rec: &mut TaskRecord, base: &mut TaskRecord, key: &str) -> Result<(), FabrError> {
    let state = state_name(rec.state);
    append_log(&mut rec.log, "notify", &format!("ignored request {key} in state {state}"));
    match key {
        "cancel" => rec.requests.cancel = false,
        "delegate" => rec.requests.delegate = false,
        "redelegate" => rec.requests.redelegate = false,
        "redo" => rec.requests.redo = None,
        _ => {}
    }
    persist(path, rec, base)
}

fn persist(path: &Path, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    update_record(path, rec, base)
}

#[cfg(test)]
mod answers_in_context {
    use super::*;
    use fabr_core::taskrec::{Answer, Grant, Prior};

    fn answered(id: &str, kind: &str, text: &str, answer: Answer) -> Question {
        Question {
            id: id.into(),
            kind: kind.into(),
            text: text.into(),
            options: None,
            prior: None,
            fields: None,
            asked_at: None,
            answer: Some(answer),
        }
    }

    fn answer(value: Option<&str>) -> Answer {
        Answer { value: value.map(Into::into), replacement: None, granted: None, by: Some("gui".into()), at: None }
    }

    #[test]
    fn choice_answers_carry_the_prior_and_grant_answers_carry_the_fields() {
        let mut rec = crate::link::test_reminder("order");
        rec.context = "base context".into();
        let mut choice = answered("q1", "choice", "Reuse the shop?", answer(Some("always")));
        choice.prior = Some(Prior { text: "shop.example.test".into(), source_task: None, last_confirmed: None });
        let mut no = answered("q2", "choice", "Reuse the size?", answer(Some("no")));
        no.answer.as_mut().unwrap().replacement = Some("2 kg".into());
        let mut grant = answered("q3", "grant", "May I fill the address?", answer(None));
        grant.answer.as_mut().unwrap().granted = Some(vec![Grant { field: "address.zip".into(), scope: "task".into() }]);
        rec.answers = vec![choice, no];
        rec.question = Some(grant);
        let ctx = context_with_answers(&rec);
        assert!(ctx.starts_with("base context"));
        assert!(ctx.contains("Already asked: Reuse the shop?\nUser answered: always (earlier choice: shop.example.test)"), "{ctx}");
        assert!(ctx.contains("User answered: no; use instead: 2 kg"), "{ctx}");
        assert!(ctx.contains("User answered: granted address.zip (task)"), "{ctx}");
    }

    #[test]
    fn question_ids_do_not_repeat() {
        let mut rec = crate::link::test_reminder("order");
        assert_eq!(next_question_id(&rec), "q1");
        rec.answers = vec![answered("q1", "open", "a?", answer(Some("x"))), answered("q2", "open", "b?", answer(Some("y")))];
        assert_eq!(next_question_id(&rec), "q3");
    }

    #[test]
    fn denied_rules_are_the_ones_logged_right_before_the_question() {
        let log = "- 2026-10-30T00:00:00Z execute: denied Bash(ls:*)\n- 2026-10-30T00:00:01Z execute: asked q1\n\
                   - 2026-10-30T00:00:02Z execute: Edit a.md\n- 2026-10-30T00:00:03Z execute: denied Bash(npm install:*)\n\
                   - 2026-10-30T00:00:03Z execute: denied Task (no rule fabr can grant)\n- 2026-10-30T00:00:03Z execute: denied WebSearch\n\
                   - 2026-10-30T00:00:04Z execute: asked q2\n";
        assert_eq!(denied_rules(log, "q1"), ["Bash(ls:*)"]);
        assert_eq!(denied_rules(log, "q2"), ["Bash(npm install:*)", "WebSearch"]);
        assert!(denied_rules(log, "q3").is_empty());
    }

    #[test]
    fn headings_are_demoted_below_the_record_sections() {
        assert_eq!(
            demote("# Title\n## Plan\n### Inputs\nplain #tag\n####### deep", 3),
            "### Title\n### Plan\n### Inputs\nplain #tag\n####### deep"
        );
        assert_eq!(missing_sections("### Inputs\nx\n### outputs\n### Steps\n1. go"), Vec::<&str>::new());
        assert_eq!(missing_sections("### Steps\n1."), ["### Inputs", "### Outputs"]);
    }

    #[test]
    fn gather_walks_the_folder_chain_nearest_first_and_stays_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let node = root.join("a/b");
        std::fs::create_dir_all(&node).unwrap();
        let agents = |dir: &Path, file: &str, text: &str| {
            let d = dir.join(".fabric-space/.agents");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(file), text).unwrap();
        };
        agents(&root, "rules.md", "## Root rule\nbe brief");
        agents(&node, "rules.md", "node rule wins");
        agents(&root.join("a"), "knowledge.md", "the printer is in room 2");
        agents(&tmp.path().join("outside"), "rules.md", "never seen");
        agents(tmp.path(), "rules.md", "above the root");
        std::fs::write(node.join("notes.md"), "# Notes\nmore").unwrap();
        std::fs::create_dir_all(node.join("sub")).unwrap();
        let out = gather_bundle(&node, Some(&root));
        let rules = out.find("#### Rules").unwrap();
        let near = out.find("From a/b (this folder):\nnode rule wins").unwrap();
        let far = out.find("From .:\n##### Root rule\nbe brief").unwrap();
        assert!(rules < near && near < far, "{out}");
        assert!(out.contains("#### Knowledge\nFrom a:\nthe printer is in room 2"), "{out}");
        assert!(out.contains("#### Files\nnotes.md: Notes\nsub/"), "{out}");
        assert!(!out.contains("never seen") && !out.contains("above the root"), "{out}");
        assert!(!out.lines().any(|l| l.starts_with("## ")), "{out}");
        agents(&node, "knowledge.md", &"x".repeat(40_000));
        let big = gather_bundle(&node, Some(&root));
        assert!(big.len() < 34 * 1024 && big.contains("(cut at 8192 bytes)"), "{}", big.len());
        assert_eq!(folder_chain(&node, None), vec![node.canonicalize().unwrap()]);
        assert_eq!(folder_chain(&node, Some(&tmp.path().join("outside"))).len(), 1);
    }
}
