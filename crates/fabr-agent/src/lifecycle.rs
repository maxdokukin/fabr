use crate::board::{Activity, Board, Extras};
use crate::caps::{Harness, requires_of};
use crate::link::{list_records, load_record, update_record};
use crate::model::{Block, EvaluateInput, ExecuteInput, Judgement, Message, Model, PlanInput, TurnOutcome};
use crate::now_utc;
use crate::state::{append_log, clean_message, is_terminal, pending_tail, question_open, stage_label, state_name, suspended_now};
use crate::store::mutate;
use crate::toolset::{Only, ReadOnly, ToolSet, default_tools};
use crate::verify;
use fabr_core::FabrError;
use fabr_core::taskrec::{
    Approval, Assertion, Author, Check, PlanMeta, Question, RedoStage, State, Step, TaskRecord, Verdict, VerdictItem, approval_digest,
    registry_filename,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_STAGE_ERRORS: u32 = 3;
const TOOL_LINE_PREFIXES: [&str; 7] = ["read_file ", "write_file ", "list_dir ", "make_dir ", "turn ", "tool failed", "asked "];

pub struct Ctl {
    pub board: Arc<Board>,
    pub check_timeout_s: u64,
    pub execute_max_turns: u32,
    pub evaluate_max_reads: usize,
    pub harness: Harness,
    pub tools: Arc<dyn ToolSet>,
}

impl Ctl {
    pub fn new(space: &Path, heartbeat_interval_s: u64) -> Ctl {
        Ctl {
            board: Board::new(space, heartbeat_interval_s, None),
            check_timeout_s: verify::DEFAULT_CHECK_TIMEOUT_S,
            execute_max_turns: crate::settings::DEFAULT_EXECUTE_MAX_TURNS,
            evaluate_max_reads: crate::settings::DEFAULT_EVALUATE_MAX_READS,
            harness: Harness::standard(),
            tools: Arc::new(crate::sponsor_tools::WorkspaceTools::new(space)),
        }
    }

    pub fn with_board(board: Arc<Board>, check_timeout_s: u64) -> Ctl {
        let tools = Arc::new(crate::sponsor_tools::WorkspaceTools::new(board.space()));
        Ctl {
            board,
            check_timeout_s,
            execute_max_turns: crate::settings::DEFAULT_EXECUTE_MAX_TURNS,
            evaluate_max_reads: crate::settings::DEFAULT_EVALUATE_MAX_READS,
            harness: Harness::standard(),
            tools,
        }
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

pub fn validate_plan(meta: &PlanMeta) -> Result<(), FabrError> {
    let mut known = default_tools().names();
    for name in crate::tools_contract::executable_names() {
        if !known.iter().any(|have| have == name) {
            known.push(name.to_string());
        }
    }
    for tool in &meta.tools {
        if !known.contains(tool) {
            return Err(FabrError::BadRequest(format!("unsupported tool {tool}")));
        }
    }
    for check in &meta.checks {
        verify::validate_check(check)?;
    }
    Ok(())
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

pub fn trailing_error(log: &str, label: &str) -> Option<ErrorLine> {
    for line in log.lines().rev().filter(|l| !l.trim().is_empty()) {
        if let Some(err) = parse_error_line(line) {
            return (err.label == label).then_some(err);
        }
        let body = line.strip_prefix("- ").unwrap_or(line);
        let is_tool_line = body
            .split_once(' ')
            .and_then(|(_, rest)| rest.strip_prefix(label))
            .and_then(|rest| rest.strip_prefix(": "))
            .is_some_and(|msg| TOOL_LINE_PREFIXES.iter().any(|p| msg.starts_with(p)));
        if !is_tool_line {
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
        if count < MAX_STAGE_ERRORS {
            return None;
        }
        r.state = State::Failed;
        r.step = None;
        r.waiting = false;
        r.verdict = None;
        r.requests.delegate = false;
        append_log(&mut r.log, label, &format!("failed after {count} errors: {message}"));
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
    model: &impl Model,
) -> Result<(), FabrError> {
    let ctl = Ctl::new(fabric_space, heartbeat_interval_s);
    advance_with(node_dir, record_path, &ctl, model)
}

pub fn advance_with(node_dir: &Path, record_path: &Path, ctl: &Ctl, model: &impl Model) -> Result<(), FabrError> {
    let rec = load_record(record_path)?;
    match step_once(node_dir, record_path, ctl, model, rec) {
        Ok(()) => Ok(()),
        Err(err) => stage_error(record_path, ctl, err),
    }
}

fn step_once(node_dir: &Path, record_path: &Path, ctl: &Ctl, model: &impl Model, mut rec: TaskRecord) -> Result<(), FabrError> {
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
            Some(Step::Define) => define_stage(node_dir, record_path, ctl, &mut rec, &mut base, model),
            Some(Step::Plan) => plan_stage(node_dir, record_path, ctl, &mut rec, &mut base, model, false),
            _ => gather_stage(node_dir, record_path, ctl, &mut rec, &mut base),
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
                return plan_stage(node_dir, record_path, ctl, &mut rec, &mut base, model, true);
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
            if !harness_gate(node_dir, record_path, ctl, &mut rec, &mut base, "execute")? {
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
            append_log(&mut rec.log, "execute", "started");
            persist(record_path, &mut rec, &mut base)?;
            mark(ctl, node_dir, record_path, &rec, Activity::Tool);
            Ok(())
        }
        State::InProgress => {
            if !matches!(rec.step, Some(Step::Learn | Step::Notify)) {
                let label = stage_label(&rec);
                if !harness_gate(node_dir, record_path, ctl, &mut rec, &mut base, label)? {
                    return Ok(());
                }
            }
            match rec.step {
                Some(Step::Evaluate) => finish_checks(node_dir, record_path, ctl, &mut rec, &mut base, model),
                Some(Step::Learn | Step::Notify) => tail_step(node_dir, record_path, ctl, &mut rec, &mut base),
                _ => execute_stage(node_dir, record_path, ctl, &mut rec, &mut base, model),
            }
        }
        State::Succeeded | State::Failed => match rec.step {
            Some(Step::Learn | Step::Notify) => tail_step(node_dir, record_path, ctl, &mut rec, &mut base),
            _ => {
                forget(ctl, &rec, record_path);
                Ok(())
            }
        },
    }
}

fn auto_approvable(ctl: &Ctl, rec: &TaskRecord) -> bool {
    ctl.harness.auto_mode
        && rec.needs.is_none()
        && rec.plan_meta.as_ref().is_some_and(|m| m.reversibility.as_deref() != Some("irreversible") && validate_plan(m).is_ok())
}

fn harness_gate(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    label: &str,
) -> Result<bool, FabrError> {
    let Some(meta) = rec.plan_meta.clone() else { return Ok(true) };
    match ctl.harness.missing_with(&meta, &ctl.tools.names()) {
        None => {
            if rec.needs.is_some() {
                rec.needs = None;
                persist(record_path, rec, base)?;
            }
            Ok(true)
        }
        Some(missing) => {
            let needs = format!("harness: {missing}");
            if rec.needs.as_deref() != Some(needs.as_str()) {
                rec.needs = Some(needs);
                append_log(&mut rec.log, label, &format!("needs harness: {missing}"));
                persist(record_path, rec, base)?;
            }
            mark(ctl, node_dir, record_path, rec, Activity::Parked);
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

fn tail_step(node_dir: &Path, record_path: &Path, ctl: &Ctl, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    if rec.step == Some(Step::Learn) {
        rec.step = Some(Step::Notify);
        append_log(&mut rec.log, "learn", "deferred");
        persist(record_path, rec, base)?;
        mark(ctl, node_dir, record_path, rec, Activity::Tool);
        return Ok(());
    }
    let result = state_name(rec.state);
    append_log(&mut rec.log, "notify", &format!("result {result}"));
    rec.step = None;
    persist(record_path, rec, base)?;
    forget(ctl, rec, record_path);
    Ok(())
}

fn gather_stage(node_dir: &Path, record_path: &Path, ctl: &Ctl, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    let mut view = rec.clone();
    view.step = Some(Step::Gather);
    mark(ctl, node_dir, record_path, &view, Activity::Tool);
    if rec.context.trim().is_empty() {
        rec.by.context = Some(Author::Agent);
    }
    append_gathered(&mut rec.context, &gather_bundle(node_dir));
    rec.step = Some(Step::Define);
    rec.waiting = false;
    append_log(&mut rec.log, "gather", "context bundle written");
    persist(record_path, rec, base)?;
    mark(ctl, node_dir, record_path, rec, Activity::Tool);
    Ok(())
}

fn gather_bundle(node_dir: &Path) -> String {
    let mut lines = Vec::new();
    if let Ok(rd) = std::fs::read_dir(node_dir) {
        let mut names: Vec<_> = rd.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for ent in names {
            let name = ent.file_name().to_string_lossy().to_string();
            if name == ".fabr" || name == ".fabric-space" || name.starts_with('.') {
                continue;
            }
            let path = ent.path();
            if path.is_file() && ent.metadata().is_ok_and(|m| m.len() <= 4096) {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let preview = text.lines().next().unwrap_or("").trim();
                lines.push(format!("{name}: {preview}"));
            } else {
                lines.push(name);
            }
        }
    }
    let body = if lines.is_empty() { "(empty)".to_string() } else { lines.join("\n") };
    format!("### Gathered\n{body}\n")
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

fn define_stage(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    model: &impl Model,
) -> Result<(), FabrError> {
    if rec.waiting && question_open(rec) {
        return Ok(());
    }
    if rec.question.as_ref().is_some_and(|q| q.answer.is_some())
        && let Some(q) = rec.question.take()
    {
        rec.answers.push(q);
    }
    let spec_human = rec.by.spec == Some(Author::Human);
    let success_human = rec.by.success == Some(Author::Human);
    if spec_human && success_human && !rec.assertions.is_empty() {
        rec.question = None;
        rec.waiting = false;
        rec.step = Some(Step::Plan);
        append_log(&mut rec.log, "define", "spec kept");
        persist(record_path, rec, base)?;
        mark(ctl, node_dir, record_path, rec, Activity::Model);
        return Ok(());
    }
    mark(ctl, node_dir, record_path, rec, Activity::Model);
    let draft = model.define(&rec.title, &context_with_answers(rec))?;
    if let Some(text) = draft.question.filter(|q| !q.trim().is_empty()) {
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
        rec.step = Some(Step::Define);
        append_log(&mut rec.log, "define", &format!("asked {id}"));
        persist(record_path, rec, base)?;
        mark(ctl, node_dir, record_path, rec, Activity::Model);
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
        rec.spec = draft.spec;
        rec.by.spec = Some(Author::Agent);
    }
    if !success_human {
        rec.success = draft.success;
        rec.by.success = Some(Author::Agent);
    }
    if !keep_assertions {
        check_assertion_count(&draft.assertions)?;
        rec.assertions = draft.assertions.into_iter().map(|a| Assertion { id: a.id, text: a.text }).collect();
    }
    let mut requested: Vec<String> = Vec::new();
    for field in draft.grants {
        if !requested.contains(&field) {
            requested.push(field);
        }
    }
    rec.grants.requested = requested;
    rec.question = None;
    rec.waiting = false;
    rec.step = Some(Step::Plan);
    append_log(&mut rec.log, "define", "spec written");
    persist(record_path, rec, base)?;
    mark(ctl, node_dir, record_path, rec, Activity::Model);
    Ok(())
}

fn plan_stage(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    model: &impl Model,
    refused: bool,
) -> Result<(), FabrError> {
    let note = if refused { rec.approval.as_ref().and_then(|a| a.note.clone()) } else { None };
    let mut view = rec.clone();
    view.step = Some(Step::Plan);
    mark(ctl, node_dir, record_path, &view, Activity::Model);
    let allowed = ctl.harness.planning();
    let answers = answers_text(rec);
    let tools_note = crate::tools_contract::planner_view(&crate::tools_contract::load_sponsor_keys(ctl.board.space()));
    let ask = |rejection: Option<&str>| {
        model.plan(&PlanInput {
            title: &rec.title,
            spec: &rec.spec,
            success: &rec.success,
            context: &rec.context,
            refusal_note: note.as_deref(),
            answers: &answers,
            allowed: &allowed,
            rejection,
            tools_note: &tools_note,
        })
    };
    let human_plan = rec.by.plan == Some(Author::Human) && !rec.plan.trim().is_empty();
    let kept = if human_plan { rec.plan_meta.clone().filter(|m| !m.checks.is_empty()) } else { None };
    let (mut meta, draft) = match kept {
        Some(meta) => (meta, None),
        None => {
            let mut draft = ask(None)?;
            check_assertion_count(&draft.assertions)?;
            let mut meta = meta_of(&draft, human_plan)?;
            ground_checks(node_dir, &mut meta, &draft.plan, &mut rec.log);
            if let Err(e) = validate_plan(&meta) {
                let rejection =
                    format!("{}; allowed tools: {}; allowed checks: {}", e.message(), allowed.tools.join(", "), allowed.checks.join(", "));
                append_log(&mut rec.log, "plan", &format!("retry after rejection: {}", clean_message(&e.message(), 200)));
                draft = ask(Some(&rejection))?;
                check_assertion_count(&draft.assertions)?;
                meta = meta_of(&draft, human_plan)?;
                ground_checks(node_dir, &mut meta, &draft.plan, &mut rec.log);
            }
            (meta, Some(draft))
        }
    };
    if draft.is_some() {
        for bound in model.planned_bounds() {
            crate::tools_contract::bind_action(&mut meta, &bound.tool, &bound.params)?;
        }
        for action in &meta.bound_actions {
            if !action.grant_class.is_empty() && !rec.grants.requested.iter().any(|field| field == &action.grant_class) {
                rec.grants.requested.push(action.grant_class.clone());
            }
        }
    }
    for tool in meta.tools.clone() {
        if crate::tools_contract::must_bind(&tool) && !meta.bound_actions.iter().any(|action| action.tool == tool) {
            return Err(FabrError::BadRequest(format!("plan must bind parameters for {tool}")));
        }
    }
    meta.requires = requires_of(&meta);
    let invalid = validate_plan(&meta).err();
    if let Some(draft) = draft {
        let spec_human = rec.by.spec == Some(Author::Human);
        let success_human = rec.by.success == Some(Author::Human);
        if !spec_human && !draft.spec.trim().is_empty() {
            rec.spec = draft.spec;
            rec.by.spec = Some(Author::Agent);
        }
        if !success_human && !draft.success.trim().is_empty() {
            rec.success = draft.success;
            rec.by.success = Some(Author::Agent);
        }
        if !spec_human && !success_human && !draft.assertions.is_empty() {
            rec.assertions = draft.assertions.into_iter().map(|a| Assertion { id: a.id, text: a.text }).collect();
        }
        if !human_plan {
            rec.plan = draft.plan;
            rec.by.plan = Some(Author::Agent);
        }
    }
    rec.needs = None;
    if let Some(err) = invalid {
        let missing = ctl.harness.missing_with(&meta, &ctl.tools.names()).unwrap_or_else(|| clean_message(&err.message(), 160));
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
    persist(record_path, rec, base)?;
    mark(ctl, node_dir, record_path, rec, Activity::Model);
    Ok(())
}

fn check_assertion_count(list: &[crate::model::DraftAssertion]) -> Result<(), FabrError> {
    match list.len() {
        0..=3 => Ok(()),
        n => Err(FabrError::BadRequest(format!("the model returned {n} assertions; 1 to 3 are allowed"))),
    }
}

fn meta_of(draft: &crate::model::PlanDraft, plan_kept: bool) -> Result<PlanMeta, FabrError> {
    if !plan_kept && draft.plan.trim().is_empty() {
        return Err(FabrError::BadRequest("plan is incomplete".into()));
    }
    Ok(PlanMeta {
        reversibility: Some(draft.reversibility.clone()),
        tools: draft.tools.clone(),
        assumptions: Vec::new(),
        checks: draft.checks.iter().map(|c| Check { cmd: c.cmd.clone(), args: c.args.clone() }).collect(),
        requires: Default::default(),
        bound_actions: Vec::new(),
    })
}

fn cancel_now(record_path: &Path, ctl: &Ctl, rec: &mut TaskRecord, base: &mut TaskRecord) -> Result<(), FabrError> {
    rec.requests.cancel = false;
    rec.state = State::Failed;
    rec.step = None;
    rec.waiting = false;
    rec.verdict = None;
    append_log(&mut rec.log, "execute", "cancelled by user");
    persist(record_path, rec, base)?;
    forget(ctl, rec, record_path);
    Ok(())
}

fn must_stop(rec: &TaskRecord) -> bool {
    rec.requests.complete || rec.requests.pause || rec.paused || suspended_now(rec)
}

fn allowed_tools(tools: &dyn ToolSet, rec: &TaskRecord) -> Vec<String> {
    let listed = |name: &str| rec.plan_meta.as_ref().is_none_or(|m| m.tools.iter().any(|t| t == name));
    let writes = listed("write_file");
    tools
        .specs()
        .into_iter()
        .filter(|s| match s.name.as_str() {
            _ if s.read_only => true,
            "write_file" => writes,
            "make_dir" => writes || listed("make_dir"),
            other => listed(other),
        })
        .map(|s| s.name)
        .collect()
}

fn execute_system(node_dir: &Path, rec: &TaskRecord, allowed: &[String]) -> String {
    let mut out = format!(
        "You are carrying out one task inside a single folder, using tools. Stay inside the folder. Allowed tools: {}.\n\
         Look before you act: list and read what exists, then make the change, then read it back to confirm. \
         Payment is never filled; stop and hand over. If something required is missing, call ask_user. \
         When the work is done, reply with a short plain summary and no tool call.\n\n",
        allowed.join(", ")
    );
    if allowed.iter().any(|t| t == "write_file") {
        out.push_str("write_file writes Markdown files only; create folders with make_dir first.\n\n");
    }
    out.push_str(&format!(
        "Task: {}\n\nSpec:\n{}\n\nPlan:\n{}\n\nAssertions that will be judged afterwards:\n",
        rec.title, rec.spec, rec.plan
    ));
    for a in &rec.assertions {
        out.push_str(&format!("{}: {}\n", a.id, a.text));
    }
    out.push('\n');
    out.push_str(&gather_bundle(node_dir));
    out
}

fn execute_opening(prior_log: &str, answers: &str) -> String {
    let lines: Vec<&str> = prior_log.lines().filter(|l| l.contains(" execute: ")).collect();
    let tail = lines[lines.len().saturating_sub(30)..].join("\n");
    let mut out = String::from("Begin. The folder is described in the system prompt as it is now.");
    if !tail.is_empty() {
        out.push_str(&format!("\n\nEarlier progress on this task (log tail):\n{tail}"));
    }
    if !answers.trim().is_empty() {
        out.push_str(&format!("\n\nThe user's answers so far:{answers}"));
    }
    out
}

fn execute_stage(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    model: &impl Model,
) -> Result<(), FabrError> {
    if rec.requests.cancel {
        return cancel_now(record_path, ctl, rec, base);
    }
    if must_stop(rec) || (rec.waiting && question_open(rec)) {
        return Ok(());
    }
    if rec.question.as_ref().is_some_and(|q| q.answer.is_some())
        && let Some(q) = rec.question.take()
    {
        rec.answers.push(q);
        rec.waiting = false;
    }
    if let Some(msg) = crate::tools_contract::unresolved_external_message(&rec.log) {
        append_log(&mut rec.log, "execute", &msg);
        append_log(&mut rec.log, "execute", "reconciliation required; external result is UNKNOWN");
        rec.step = Some(Step::Evaluate);
        rec.verdict = None;
        persist(record_path, rec, base)?;
        mark(ctl, node_dir, record_path, rec, Activity::Tool);
        return Ok(());
    }
    let prior_log = rec.log.clone();
    let allowed = allowed_tools(&*ctl.tools, rec);
    let view = Only::new(&*ctl.tools, allowed.clone());
    let system = execute_system(node_dir, rec, &allowed);
    let mut messages = vec![Message::user(vec![Block::Text(execute_opening(&prior_log, &answers_text(rec)))])];
    let mut finished = false;
    for turn in 1..=ctl.execute_max_turns {
        mark(ctl, node_dir, record_path, rec, Activity::Model);
        let out = model.execute_turn(&ExecuteInput { system: &system, tools: &view }, &mut messages)?;
        append_log(&mut rec.log, "execute", &format!("turn {turn}"));
        match out.outcome {
            TurnOutcome::Done(_) => {
                persist(record_path, rec, base)?;
                finished = true;
                break;
            }
            TurnOutcome::Ask(text) => {
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
                append_log(&mut rec.log, "execute", &format!("asked {id}"));
                persist(record_path, rec, base)?;
                mark(ctl, node_dir, record_path, rec, Activity::Parked);
                return Ok(());
            }
            TurnOutcome::Tools(uses) => {
                let mut results = Vec::new();
                for u in uses {
                    mark(ctl, node_dir, record_path, rec, Activity::Tool);
                    let call = &u.call;
                    let skip = call.name == "write_file" && log_has_write(&prior_log, &call.path);
                    let allowed_here = allowed.contains(&call.name);
                    let (content, is_error) = if skip {
                        ("skipped: this file was already written earlier in the task".to_string(), false)
                    } else if !allowed_here {
                        append_log(&mut rec.log, "execute", &format!("tool failed: {} not allowed", call.name));
                        (format!("tool {} is not allowed", call.name), true)
                    } else if let Some(external) = external_call(node_dir, record_path, ctl, rec, base, &view, call)? {
                        if external.stop {
                            return Ok(());
                        }
                        (external.content, external.is_error)
                    } else {
                        match view.call(node_dir, call) {
                            Ok(result) => {
                                let line = if matches!(call.name.as_str(), "read_file" | "write_file" | "list_dir" | "make_dir") {
                                    format!("{} {}", call.name, call.path).trim_end().to_string()
                                } else {
                                    clean_message(&result, 400)
                                };
                                append_log(&mut rec.log, "execute", &line);
                                (result, false)
                            }
                            Err(e) => {
                                let msg = clean_message(&e, 160);
                                append_log(&mut rec.log, "execute", &format!("tool failed: {} {}: {msg}", call.name, call.path));
                                (e, true)
                            }
                        }
                    };
                    persist(record_path, rec, base)?;
                    mark(ctl, node_dir, record_path, rec, Activity::Tool);
                    results.push(Block::ToolResult { id: u.id, content, is_error });
                    if rec.requests.cancel || must_stop(rec) {
                        return Ok(());
                    }
                }
                messages.push(Message::user(results));
            }
        }
        persist(record_path, rec, base)?;
        if rec.requests.cancel {
            return cancel_now(record_path, ctl, rec, base);
        }
        if must_stop(rec) {
            return Ok(());
        }
    }
    if !finished {
        append_log(&mut rec.log, "execute", "turn cap reached");
        persist(record_path, rec, base)?;
        return Err(FabrError::BadRequest(format!("turn cap reached ({} turns)", ctl.execute_max_turns)));
    }
    rec.step = Some(Step::Evaluate);
    rec.verdict = None;
    append_log(&mut rec.log, "execute", "tools finished");
    persist(record_path, rec, base)?;
    mark(ctl, node_dir, record_path, rec, Activity::Check);
    Ok(())
}

fn finish_checks(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    model: &impl Model,
) -> Result<(), FabrError> {
    let checks = rec.plan_meta.as_ref().map(|m| m.checks.clone()).unwrap_or_default();
    mark(ctl, node_dir, record_path, rec, Activity::Check);
    let outcomes = verify::run_checks(node_dir, &checks, Duration::from_secs(ctl.check_timeout_s))?;
    let (items, pass) = if rec.assertions.is_empty() {
        let items: Vec<VerdictItem> = outcomes
            .iter()
            .enumerate()
            .map(|(i, o)| VerdictItem { id: format!("c{}", i + 1), pass: o.pass, evidence: o.evidence.clone() })
            .collect();
        let pass = !items.is_empty() && items.iter().all(|i| i.pass);
        (items, pass)
    } else {
        mark(ctl, node_dir, record_path, rec, Activity::Model);
        let lines: Vec<&str> = rec.log.lines().filter(|l| l.contains(" execute: ")).collect();
        let log = lines[lines.len().saturating_sub(80)..].join("\n");
        let read_only = ReadOnly(&*ctl.tools);
        let judged = model.evaluate(&EvaluateInput {
            title: &rec.title,
            assertions: &rec.assertions,
            log: &log,
            checks: &outcomes,
            node_dir,
            tools: &read_only,
            max_reads: ctl.evaluate_max_reads,
        })?;
        let items: Vec<VerdictItem> = rec
            .assertions
            .iter()
            .map(|a| match judged.items.iter().find(|i| i.id == a.id) {
                Some(i) => VerdictItem {
                    id: a.id.clone(),
                    pass: i.judgement == Judgement::Pass,
                    evidence: match i.judgement {
                        Judgement::Unchecked => format!("unchecked: {}", i.evidence),
                        _ => i.evidence.clone(),
                    },
                },
                None => VerdictItem { id: a.id.clone(), pass: false, evidence: "unchecked: no verdict returned".into() },
            })
            .collect();
        let pass = items.iter().all(|i| i.pass);
        (items, pass)
    };
    let unknown = crate::tools_contract::log_has_unknown(&rec.log);
    let pass = pass && !unknown;
    rec.verdict = Some(Verdict { result: if pass { "pass" } else { "fail" }.into(), at: now_utc(), items });
    rec.state = if pass { State::Succeeded } else { State::Failed };
    rec.step = Some(Step::Learn);
    rec.waiting = false;
    let evaluate_line = if pass {
        "verdict pass"
    } else if unknown {
        "verdict fail; external UNKNOWN requires reconciliation"
    } else {
        "verdict fail"
    };
    append_log(&mut rec.log, "evaluate", evaluate_line);
    persist(record_path, rec, base)?;
    mark(ctl, node_dir, record_path, rec, Activity::Tool);
    Ok(())
}

fn ground_checks(node_dir: &Path, meta: &mut PlanMeta, plan: &str, log: &mut String) {
    let created = |file: &str| {
        plan.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.contains(&file.to_ascii_lowercase())
                && ["create", "write", "add", "new", "save", "make", "generate", "append"].iter().any(|v| l.contains(v))
        })
    };
    meta.checks.retain(|c| {
        let files = verify::file_args(c);
        let unknown = !files.is_empty() && files.iter().all(|f| !node_dir.join(f).exists() && !created(f));
        if unknown {
            append_log(log, "plan", &format!("dropped check {} {}: unknown file", c.cmd, c.args.join(" ")));
        }
        !unknown
    });
}

struct ExternalReport {
    content: String,
    is_error: bool,
    stop: bool,
}

fn external_call(
    node_dir: &Path,
    record_path: &Path,
    ctl: &Ctl,
    rec: &mut TaskRecord,
    base: &mut TaskRecord,
    view: &dyn ToolSet,
    call: &crate::model::ToolCall,
) -> Result<Option<ExternalReport>, FabrError> {
    let params = crate::tools_contract::call_params(call);
    if crate::tools_contract::in_catalog(&call.name) {
        let keys = crate::tools_contract::load_sponsor_keys(ctl.board.space());
        let approved = rec.plan_meta.as_ref().map(|m| m.tools.clone()).unwrap_or_default();
        let bound = rec.plan_meta.as_ref().map(|m| m.bound_actions.clone()).unwrap_or_default();
        crate::tools_contract::execution_allowed(&call.name, &keys, &rec.grants.granted, &approved, &bound, &params)?;
    }
    if !crate::tools_contract::must_bind(&call.name) {
        return Ok(None);
    }
    let id = crate::tools_contract::invocation_id(&rec.id, &call.name, &params);
    if crate::tools_contract::external_replay(&rec.log, &id, crate::tools_contract::Idempotency::Never)
        == crate::tools_contract::ExternalReplay::DoNotReplay
    {
        if let Some(msg) = crate::tools_contract::unresolved_external_message(&rec.log) {
            return Ok(Some(park_unknown(record_path, rec, base, &msg)?));
        }
        return Err(FabrError::BadRequest(format!("external {} is not replayed", call.name)));
    }
    append_log(&mut rec.log, "execute", &crate::tools_contract::external_intent_message(&id, &call.name));
    persist(record_path, rec, base)?;
    match view.call(node_dir, call) {
        Ok(text) if unknown_text(&text) => Ok(Some(park_unknown(
            record_path,
            rec,
            base,
            &crate::tools_contract::external_unknown_message(&id, &call.name, unknown_label(&text)),
        )?)),
        Ok(text) => {
            let evidence = clean_message(&text, 180);
            append_log(&mut rec.log, "execute", &format!("external {id} {} succeeded {evidence}", call.name));
            persist(record_path, rec, base)?;
            Ok(Some(ExternalReport { content: text, is_error: false, stop: false }))
        }
        Err(err) if unknown_text(&err) => Ok(Some(park_unknown(
            record_path,
            rec,
            base,
            &crate::tools_contract::external_unknown_message(&id, &call.name, unknown_label(&err)),
        )?)),
        Err(err) => {
            let msg = clean_message(&err, 160);
            append_log(&mut rec.log, "execute", &format!("external {id} {} failed {msg}", call.name));
            persist(record_path, rec, base)?;
            Ok(Some(ExternalReport { content: msg, is_error: true, stop: false }))
        }
    }
}

fn unknown_text(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower == "timeout" || lower == "unavailable" || lower.starts_with("timeout ") || lower.starts_with("unavailable ")
}

fn unknown_label(text: &str) -> &'static str {
    if text.to_ascii_lowercase().contains("timeout") { "timeout" } else { "unavailable" }
}

fn park_unknown(record_path: &Path, rec: &mut TaskRecord, base: &mut TaskRecord, message: &str) -> Result<ExternalReport, FabrError> {
    append_log(&mut rec.log, "execute", message);
    append_log(&mut rec.log, "execute", "reconciliation required; external result is UNKNOWN");
    rec.step = Some(Step::Evaluate);
    rec.verdict = None;
    persist(record_path, rec, base)?;
    Ok(ExternalReport { content: message.to_string(), is_error: true, stop: true })
}

fn log_has_write(log: &str, path: &str) -> bool {
    let needle = format!("execute: write_file {path}");
    log.lines().any(|line| line.contains(&needle))
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
}
