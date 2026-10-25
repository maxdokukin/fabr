mod calls;
mod events;
pub(crate) mod heartbeat;
mod registry;
mod signal;

pub use calls::{CallLog, Sink, StepUsage, cost_usd};
pub(crate) use heartbeat::write_atomic;
pub use heartbeat::write_failure;
pub use signal::install as install_signals;

use crate::board::{Activity, Board, Extras};
use crate::lifecycle::{Ctl, advance_with, apply_redo, task_path_key, trailing_error};
use crate::model::AnthropicModel;
use crate::now_utc;
use crate::settings::check_timeout_s;
use crate::state::{
    append_log, clean_message, driven, is_terminal, parked, pending_tail, stage_label, state_name, suspend_expired, suspended_now,
};
use crate::store::mutate;
use chrono::Utc;
use events::Events;
use fabr_core::fabric::{self, FabricListReq};
use fabr_core::redact::{fingerprint, register_secret};
use fabr_core::setup::{Ctx, ExecuteMode, read_secret, settings_read};
use fabr_core::taskrec::{Grants, Links, Requests, State, TaskRecord, parse, write_record};
use fabr_core::{FabrError, Secret, Workspace};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const RETRY_AFTER_TICKS: u64 = 6;
const RETRY_CAP_S: u64 = 600;
const FRESH_EVENT_WINDOW_S: i64 = 24 * 3600;

#[derive(Debug, Clone)]
pub struct Options {
    pub interval_s: u64,
    pub once: bool,
}

pub struct Gate {
    pub default_folder: PathBuf,
    pub model: String,
    pub key: Secret,
    pub heartbeat_interval_s: u64,
}

pub fn start_gate(ctx: &Ctx) -> Result<Gate, FabrError> {
    let refuse = |m: &str| Err(FabrError::BadRequest(m.to_string()));
    let read = settings_read(ctx)?;
    if !read.exists {
        return refuse("Setup has not been started, so open the app and finish the setup screens first.");
    }
    if !read.settings.setup.completed {
        return refuse("Setup is not complete, so finish the setup screens in the app before starting the agent.");
    }
    let Some(key) = read_secret(ctx, "anthropic_api_key") else {
        return refuse("No Anthropic API key is saved, so enter it in the app's setup first.");
    };
    if read.settings.model.trim().is_empty() {
        return refuse("No model is selected, so pick one in the app's setup first.");
    }
    let folder = PathBuf::from(&read.settings.default_folder);
    if !folder.is_dir() {
        return Err(FabrError::BadRequest(format!(
            "The default folder {} does not exist, so pick another one in setup.",
            folder.display()
        )));
    }
    register_secret(&key, &fingerprint(&key));
    if let Some(token) = read_secret(ctx, "telegram_bot_token") {
        register_secret(&token, "[bot token]");
    }
    Ok(Gate {
        default_folder: folder,
        model: read.settings.model.trim().to_string(),
        key: Secret::new(key),
        heartbeat_interval_s: u64::from(read.settings.heartbeat_interval_s).clamp(1, 60),
    })
}

pub struct Runner {
    space: PathBuf,
    root: PathBuf,
    model: AnthropicModel,
    calls: Arc<CallLog>,
    board: Arc<Board>,
    ctl: Ctl,
    opts: Options,
    sink: Sink,
    events: Events,
    gate_model: String,
    fingerprint: String,
    hb_interval_s: u64,
    retry_base_s: u64,
    beating: Option<heartbeat::Beating>,
    emitted: HashSet<String>,
    seeded: bool,
    event_cutoff: Option<String>,
    stuck_seen: HashSet<String>,
    bad_records: HashSet<PathBuf>,
    suspended_seen: HashSet<PathBuf>,
    known: HashSet<PathBuf>,
}

impl Runner {
    pub fn new(ctx: &Ctx, _ws: Workspace, opts: Options, sink: Sink) -> Result<Runner, FabrError> {
        let gate = start_gate(ctx)?;
        let fp = fingerprint(gate.key.expose());
        let calls = CallLog::new(sink.clone(), fp.clone());
        let model =
            AnthropicModel::new(gate.key.clone(), gate.model.clone()).with_base_url(&ctx.anthropic_url).with_observer(calls.observer());
        let space = ctx.space();
        let board = Board::new(&space, gate.heartbeat_interval_s, Some(calls.clone()));
        let mut ctl = Ctl::with_board(board.clone(), check_timeout_s(&space));
        ctl.harness = crate::caps::load(ctx)?;
        (ctl.execute_max_turns, ctl.evaluate_max_reads) = crate::settings::limits(&space);
        Ok(Runner {
            events: Events::new(&space),
            space,
            root: gate.default_folder,
            model,
            calls,
            board,
            ctl,
            retry_base_s: opts.interval_s.max(1) * RETRY_AFTER_TICKS,
            opts,
            sink,
            gate_model: gate.model,
            fingerprint: fp,
            hb_interval_s: gate.heartbeat_interval_s,
            beating: None,
            emitted: HashSet::new(),
            seeded: false,
            event_cutoff: None,
            stuck_seen: HashSet::new(),
            bad_records: HashSet::new(),
            suspended_seen: HashSet::new(),
            known: HashSet::new(),
        })
    }

    pub fn with_retry_base_s(mut self, seconds: u64) -> Runner {
        self.retry_base_s = seconds;
        self
    }

    pub fn with_model_retry_base(mut self, base: Duration) -> Runner {
        self.model = self.model.with_retry_base(base);
        self
    }

    pub fn with_check_timeout_s(mut self, seconds: u64) -> Runner {
        self.ctl.check_timeout_s = seconds;
        self
    }

    pub fn start(&mut self) -> Result<(), FabrError> {
        let eff = self.ctl.harness.effective();
        self.sink.info(format!(
            "execute={} tools=[{}] checks=[{}]",
            self.ctl.harness.mode.as_str(),
            eff.tools.join(","),
            eff.checks.join(",")
        ));
        self.sink.info(format!(
            "agent starting: default_folder={} model={} key={} interval={}s heartbeat={}s",
            self.root.display(),
            self.gate_model,
            self.fingerprint,
            self.opts.interval_s,
            self.hb_interval_s
        ));
        self.event_cutoff = self.events.newest_at();
        self.clean_stale();
        self.warn_out_of_root();
        let hb = heartbeat::Heartbeat::new(&self.space, self.hb_interval_s);
        self.beating = Some(heartbeat::Beating::spawn(hb, self.board.clone())?);
        Ok(())
    }

    pub fn run(&mut self, stop: &AtomicBool) -> Result<(), FabrError> {
        self.start()?;
        loop {
            self.tick();
            if self.opts.once || stop.load(Ordering::SeqCst) {
                break;
            }
            let mut waited = Duration::ZERO;
            while waited < Duration::from_secs(self.opts.interval_s) && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                waited += Duration::from_millis(100);
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
        }
        self.shutdown(!self.opts.once);
        Ok(())
    }

    pub fn shutdown(&mut self, clean: bool) {
        if clean {
            self.board.clear();
            let ours = std::process::id() as u64;
            for entry in registry::list(&self.space).into_iter().filter(|e| e.pid == ours) {
                let _ = std::fs::remove_file(&entry.path);
            }
        }
        if let Some(beating) = self.beating.take() {
            beating.stop(clean);
        }
        self.sink.info(if clean { "agent stopped" } else { "agent tick finished" });
    }

    fn clean_stale(&mut self) {
        let now = Utc::now();
        for entry in registry::list(&self.space) {
            if !entry.dead(now) {
                continue;
            }
            let age = entry.age_s(now).map_or("no heartbeat".to_string(), |a| format!("{a}s old"));
            self.sink.info(format!("removing stale registry entry {} ({age})", entry.path.display()));
            self.emit("stuck", &entry.task_path, &format!("agent stopped answering ({age})"));
            self.stuck_seen.insert(entry.task_path.clone());
            let _ = std::fs::remove_file(&entry.path);
        }
    }

    fn emit(&self, kind: &str, task_path: &str, detail: &str) {
        match self.events.append(kind, task_path, detail) {
            Ok(()) => self.sink.info(format!("event {kind} {task_path}")),
            Err(e) => self.sink.warn(format!("events.log append failed: {e}")),
        }
    }

    /// All task-record paths `fabric::list` discovers under `self.root`, before
    /// containment filtering. Shared by [`records`](Self::records) (which
    /// filters) and [`warn_out_of_root`](Self::warn_out_of_root) (which warns on
    /// the dropped ones) so both start from the same set.
    fn listed_records(&self) -> Vec<PathBuf> {
        let req = FabricListReq { root: self.root.to_string_lossy().into_owned(), home: None };
        match fabric::list(&req) {
            Ok(res) => res.records.into_iter().map(|f| PathBuf::from(f.path)).collect(),
            Err(e) => {
                self.sink.warn(format!("cannot list records: {}", e.message()));
                Vec::new()
            }
        }
    }

    /// The in-scope task-record paths the worker drives.
    ///
    /// A path that cannot be canonicalized is kept in scope: a transient stat
    /// failure must not drop a real record. Only a path that resolves outside
    /// the canonical root is excluded.
    fn records(&self) -> Vec<PathBuf> {
        let listed = self.listed_records();
        let Ok(root) = self.root.canonicalize() else { return listed };
        listed.into_iter().filter(|path| in_root(&root, path)).collect()
    }

    /// Warn once per record whose canonical path escapes the search root.
    /// Shares `bad_records` with [`load`](Self::load).
    fn warn_out_of_root(&mut self) {
        let Ok(root) = self.root.canonicalize() else { return };
        for path in self.listed_records() {
            if in_root(&root, &path) {
                continue;
            }
            let Ok(canon) = path.canonicalize() else { continue };
            if self.bad_records.insert(path.clone()) {
                self.sink.warn(format!(
                    "record {} resolves to {}, outside the default folder {}; excluded from driving — \
                     check for a symlink escaping the search root",
                    path.display(),
                    canon.display(),
                    root.display()
                ));
            }
        }
    }

    fn seed_events(&mut self, paths: &[PathBuf]) {
        let fresh = format!("{}Z", (Utc::now() - chrono::Duration::seconds(FRESH_EVENT_WINDOW_S)).format("%Y-%m-%dT%H:%M:%S"));
        let cutoff = self.event_cutoff.clone().unwrap_or(fresh);
        for path in paths {
            let Some(rec) = std::fs::read_to_string(path).ok().and_then(|t| parse(&t).ok()) else { continue };
            let task = task_path_key(path);
            for ev in event_keys(path, &rec) {
                if ev.at < cutoff || self.events.seen(ev.kind, &task, &ev.at) {
                    self.emitted.insert(ev.dedupe);
                }
            }
        }
    }

    pub fn tick(&mut self) {
        let now = Utc::now();
        let ours = std::process::id() as u64;
        for entry in registry::list(&self.space) {
            if entry.pid != ours && entry.dead(now) && self.stuck_seen.insert(entry.task_path.clone()) {
                let age = entry.age_s(now).map_or("no heartbeat".to_string(), |a| format!("{a}s old"));
                self.emit("stuck", &entry.task_path, &format!("agent stopped answering ({age})"));
            }
        }
        if self.ctl.harness.mode == ExecuteMode::Off {
            return;
        }
        let paths = self.records();
        if !self.seeded {
            self.seeded = true;
            self.seed_events(&paths);
        }
        for path in paths {
            self.process(&path);
        }
    }

    fn load(&mut self, path: &Path) -> Option<TaskRecord> {
        match std::fs::read_to_string(path).map_err(|e| FabrError::Io(e.to_string())).and_then(|t| parse(&t)) {
            Ok(rec) => Some(rec),
            Err(e) => {
                if self.bad_records.insert(path.to_path_buf()) {
                    self.sink.warn(format!("skipping unreadable record {}: {}", path.display(), e.message()));
                }
                None
            }
        }
    }

    fn retry_hold(&self, rec: &TaskRecord) -> Option<(u64, String)> {
        let err = trailing_error(&rec.log, stage_label(rec))?;
        let at = chrono::NaiveDateTime::parse_from_str(err.at.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S").ok()?.and_utc();
        let delay = self.retry_base_s.saturating_mul(u64::from(err.count)).min(RETRY_CAP_S) as i64;
        let left = (at + chrono::Duration::seconds(delay) - Utc::now()).num_seconds();
        (left > 0).then_some((left as u64, err.message))
    }

    fn sync(&self, path: &Path, node_dir: &Path, rec: &TaskRecord, extras: Option<Extras>) {
        let task = task_path_key(path);
        if rec.step.is_none() || (is_terminal(rec) && !pending_tail(rec)) {
            self.board.remove(&rec.id, &task);
            return;
        }
        if suspended_now(rec) {
            self.board.remove(&rec.id, &task);
            return;
        }
        let extras = extras.unwrap_or_else(|| {
            if rec.paused {
                Extras::doing(Activity::Paused)
            } else if parked(rec) || rec.needs.is_some() {
                Extras::doing(Activity::Parked)
            } else if rec.state == State::Defined {
                Extras::doing(Activity::Queued)
            } else {
                Extras::doing(Activity::Tool)
            }
        });
        if let Err(e) = self.board.publish(node_dir, path, rec, extras) {
            self.sink.warn(format!("registry write failed for {}: {}", rec.id, e.message()));
        }
    }

    fn process(&mut self, path: &Path) {
        let Some(rec) = self.load(path) else { return };
        let node_dir = node_dir_of(path);

        if rec.requests.cancel {
            let r = if rec.state == State::InProgress { self.cancel(path) } else { self.ignore(path, "cancel", rec.state) };
            self.note_err("cancel", path, r.err());
        }
        if rec.requests.complete {
            let r = if matches!(rec.state, State::Reminder | State::Task | State::Defined | State::InProgress) {
                self.complete(path)
            } else {
                self.ignore(path, "complete", rec.state)
            };
            self.note_err("complete", path, r.err());
        }
        if rec.requests.redelegate {
            if rec.state == State::Failed {
                let r = self.redelegate(path, &rec, &node_dir);
                self.note_err("redelegate", path, r.err());
            } else {
                let r = self.ignore(path, "redelegate", rec.state);
                self.note_err("redelegate", path, r.err());
            }
        }
        if let Some(stage) = rec.requests.redo {
            let r = if matches!(rec.state, State::Task | State::Defined) {
                mutate(path, |r| apply_redo(r, stage)).map(|_| ())
            } else {
                self.ignore(path, "redo", rec.state)
            };
            self.note_err("redo", path, r.err());
        }
        if rec.requests.delegate && rec.state != State::Reminder {
            let r = self.ignore(path, "delegate", rec.state);
            self.note_err("delegate", path, r.err());
        }
        let Some(mut rec) = self.control(path) else { return };
        let live = matches!(rec.state, State::Task | State::Defined | State::InProgress);
        if live && rec.question.as_ref().is_some_and(|q| q.answer.is_some()) {
            match fold_answer(path) {
                Ok(()) => self.sink.info(format!("answer received for {}; question moved to answers", rec.id)),
                Err(e) => {
                    self.sink.warn(format!("could not record the answer on {}: {}; will retry next tick", rec.id, e.message()));
                    return;
                }
            }
            match self.load(path) {
                Some(r) => rec = r,
                None => return,
            }
        }
        let drive_it = !rec.paused
            && !suspended_now(&rec)
            && match rec.state {
                State::Reminder => rec.requests.delegate,
                State::Task | State::Defined | State::InProgress => true,
                State::Succeeded | State::Failed => pending_tail(&rec),
            };
        let mut extras = None;
        if drive_it {
            if let Some((left, message)) = self.retry_hold(&rec) {
                extras = Some(Extras { activity: Some(Activity::Retry), retry_in_s: Some(left), last_error: Some(message) });
            } else {
                self.calls.begin(&rec.id);
                let r = self.drive(path, &node_dir);
                let cost = self.calls.take_cost();
                if cost > 0.0
                    && let Err(e) = mutate(path, |r| r.cost_usd += cost)
                {
                    self.sink.warn(format!("could not record cost on {}: {}", rec.id, e.message()));
                }
                if let Err(e) = r {
                    self.sink.warn(format!("{} stopped at {}: {}", rec.id, step_text(&rec), e.message()));
                    if let Some(now) = self.load(path)
                        && let Some(err) = trailing_error(&now.log, stage_label(&now))
                    {
                        let left = self.retry_base_s.saturating_mul(u64::from(err.count)).min(RETRY_CAP_S);
                        extras = Some(Extras {
                            activity: Some(Activity::Retry),
                            retry_in_s: Some(left),
                            last_error: Some(clean_message(&err.message, 300)),
                        });
                    }
                }
            }
        }
        let Some(rec) = (if drive_it { self.control(path) } else { self.load(path) }) else { return };
        self.sync(path, &node_dir, &rec, extras);
        let task = task_path_key(path);
        for ev in event_keys(path, &rec) {
            if self.emitted.insert(ev.dedupe) {
                self.emit(ev.kind, &task, &ev.detail);
            }
        }
    }

    fn control(&mut self, path: &Path) -> Option<TaskRecord> {
        let mut rec = self.load(path)?;
        if suspend_expired(&rec) {
            let r = mutate(path, |r| {
                r.suspended = false;
                r.suspend_until = None;
                append_log(&mut r.log, "unsuspended", "timer");
            });
            self.note_err("unsuspend", path, r.err());
            self.suspended_seen.remove(path);
            rec = self.load(path)?;
        }
        let is_driven = driven(&rec);
        if rec.requests.resume {
            let r = if rec.paused && is_driven {
                mutate(path, |r| {
                    let label = stage_label(r);
                    r.paused = false;
                    r.requests.resume = false;
                    append_log(&mut r.log, label, "resumed by user");
                })
                .map(|_| ())
            } else {
                self.ignore(path, "resume", rec.state)
            };
            self.note_err("resume", path, r.err());
            rec = self.load(path)?;
        }
        if rec.requests.pause {
            let r = if is_driven && !rec.paused {
                mutate(path, |r| {
                    let label = stage_label(r);
                    r.paused = true;
                    r.requests.pause = false;
                    append_log(&mut r.log, label, "paused by user");
                })
                .map(|_| ())
            } else if is_driven {
                mutate(path, |r| r.requests.pause = false).map(|_| ())
            } else {
                self.ignore(path, "pause", rec.state)
            };
            self.note_err("pause", path, r.err());
            rec = self.load(path)?;
        }
        if suspended_now(&rec) {
            if is_driven {
                self.board.remove(&rec.id, &task_path_key(path));
                let first_sight = !self.known.contains(path);
                if self.suspended_seen.insert(path.to_path_buf()) && !(first_sight && last_marker_suspended(&rec.log)) {
                    let r = mutate(path, |r| {
                        let label = stage_label(r);
                        append_log(&mut r.log, label, "suspended");
                    });
                    self.note_err("suspend", path, r.err());
                    rec = self.load(path)?;
                }
            }
        } else {
            self.suspended_seen.remove(path);
        }
        self.known.insert(path.to_path_buf());
        Some(rec)
    }

    fn note_err(&self, what: &str, path: &Path, err: Option<FabrError>) {
        if let Some(e) = err {
            self.sink.warn(format!("{what} on {} failed: {}; will retry next tick", path.display(), e.message()));
        }
    }

    fn drive(&self, path: &Path, node_dir: &Path) -> Result<(), FabrError> {
        for _ in 0..16 {
            let before = std::fs::read_to_string(path).map_err(|e| FabrError::Io(e.to_string()))?;
            let rec = parse(&before)?;
            if is_terminal(&rec) && !pending_tail(&rec) {
                break;
            }
            if parked(&rec) || rec.paused || rec.requests.pause || suspended_now(&rec) {
                break;
            }
            advance_with(node_dir, path, &self.ctl, &self.model)?;
            let after = std::fs::read_to_string(path).map_err(|e| FabrError::Io(e.to_string()))?;
            if after == before {
                break;
            }
            if let Ok(now) = parse(&after) {
                self.sync(path, node_dir, &now, None);
            }
        }
        Ok(())
    }

    fn cancel(&self, path: &Path) -> Result<(), FabrError> {
        let (rec, ()) = mutate(path, |r| {
            r.requests.cancel = false;
            r.state = State::Failed;
            r.step = None;
            r.waiting = false;
            r.paused = false;
            r.verdict = None;
            r.needs = None;
            append_log(&mut r.log, "execute", "cancelled by user");
        })?;
        self.board.remove(&rec.id, &task_path_key(path));
        self.sink.info(format!("cancelled {}", rec.id));
        Ok(())
    }

    fn complete(&self, path: &Path) -> Result<(), FabrError> {
        let (rec, ()) = mutate(path, |r| {
            let label = if r.state == State::Reminder { "done" } else { stage_label(r) };
            r.requests.complete = false;
            r.state = State::Succeeded;
            r.step = None;
            r.waiting = false;
            r.paused = false;
            r.verdict = None;
            r.needs = None;
            append_log(&mut r.log, label, "completed by user");
        })?;
        self.board.remove(&rec.id, &task_path_key(path));
        self.sink.info(format!("completed {}", rec.id));
        Ok(())
    }

    fn ignore(&self, path: &Path, key: &str, state: State) -> Result<(), FabrError> {
        let msg = format!("ignored request {key} in state {}", state_name(state));
        mutate(path, |r| {
            match key {
                "cancel" => r.requests.cancel = false,
                "delegate" => r.requests.delegate = false,
                "pause" => r.requests.pause = false,
                "resume" => r.requests.resume = false,
                "complete" => r.requests.complete = false,
                "redo" => r.requests.redo = None,
                _ => r.requests.redelegate = false,
            }
            append_log(&mut r.log, "notify", &msg);
        })?;
        self.sink.info(format!("{}: {msg}", path.display()));
        Ok(())
    }

    fn redelegate(&mut self, path: &Path, old: &TaskRecord, node_dir: &Path) -> Result<(), FabrError> {
        let existing = old.links.redelegated_to.clone().filter(|p| Path::new(p).exists());
        let new_path = match existing {
            Some(p) => PathBuf::from(p),
            None => {
                let tasks = node_dir.join(".fabric-space").join("tasks");
                let id = free_id(&tasks, &old.title);
                let now = now_utc();
                let rec = TaskRecord {
                    schema: fabr_core::taskrec::SCHEMA,
                    id: id.clone(),
                    source: old.source.clone(),
                    title: old.title.clone(),
                    due: old.due.clone(),
                    pri: old.pri,
                    state: State::Reminder,
                    step: None,
                    waiting: false,
                    paused: false,
                    suspended: false,
                    suspend_until: None,
                    created_at: now.clone(),
                    updated_at: now,
                    requests: Requests {
                        delegate: true,
                        cancel: false,
                        redelegate: false,
                        pause: false,
                        resume: false,
                        complete: false,
                        redo: None,
                    },
                    question: None,
                    answers: Vec::new(),
                    grants: Grants { requested: Vec::new(), granted: Vec::new() },
                    plan_meta: None,
                    assertions: Vec::new(),
                    approval: None,
                    verdict: None,
                    cost_usd: 0.0,
                    links: Links { redelegated_from: Some(task_path_key(path)), redelegated_to: None },
                    by: Default::default(),
                    needs: None,
                    context: old.context.clone(),
                    spec: String::new(),
                    success: String::new(),
                    plan: String::new(),
                    log: String::new(),
                };
                let new_path = tasks.join(format!("{id}.md"));
                write_record(&new_path, &rec, None)?;
                new_path
            }
        };
        let to = task_path_key(&new_path);
        mutate(path, |r| {
            r.requests.redelegate = false;
            r.links.redelegated_to = Some(to.clone());
            append_log(&mut r.log, "notify", &format!("redelegated to {to}"));
        })?;
        self.sink.info(format!("redelegated {} to {}", old.id, new_path.display()));
        self.process(&new_path);
        Ok(())
    }
}

fn last_marker_suspended(log: &str) -> bool {
    for line in log.lines().rev() {
        let Some((_, rest)) = line.strip_prefix("- ").and_then(|b| b.split_once(' ')) else { continue };
        match rest.split_once(": ") {
            Some(("unsuspended", "timer")) => return false,
            Some((label, "suspended")) if label != "unsuspended" => return true,
            _ => {}
        }
    }
    false
}

fn fold_answer(path: &Path) -> Result<(), FabrError> {
    mutate(path, |r| {
        if r.question.as_ref().is_some_and(|q| q.answer.is_some())
            && let Some(q) = r.question.take()
        {
            let id = q.id.clone();
            r.answers.push(q);
            r.waiting = r.state == State::Defined && r.approval.as_ref().is_some_and(|a| a.status == "pending");
            append_log(&mut r.log, "define", &format!("answered {id}"));
        }
    })
    .map(|_| ())
}

fn node_dir_of(record: &Path) -> PathBuf {
    record.parent().and_then(Path::parent).and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default()
}

fn free_id(tasks: &Path, title: &str) -> String {
    let base = format!("{}-{}", chrono::Local::now().format("%Y-%m-%d"), fabric::slugify(title));
    let mut id = base.clone();
    let mut n = 2;
    while tasks.join(format!("{id}.md")).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn step_text(rec: &TaskRecord) -> String {
    format!("{} {}", state_name(rec.state), rec.step.map_or("-".to_string(), |s| format!("{s:?}").to_lowercase()))
}

struct EventKey {
    dedupe: String,
    kind: &'static str,
    detail: String,
    at: String,
}

fn log_time(log: &str, labels: &[&str], needle: Option<&str>) -> Option<String> {
    log.lines().rev().find_map(|line| {
        let body = line.strip_prefix("- ")?;
        let (at, rest) = body.split_once(' ')?;
        let (label, msg) = rest.split_once(": ")?;
        (labels.contains(&label) && needle.is_none_or(|n| msg.contains(n))).then(|| at.to_string())
    })
}

fn event_keys(path: &Path, rec: &TaskRecord) -> Vec<EventKey> {
    let at = path.to_string_lossy();
    let mut out = Vec::new();
    if rec.waiting {
        if let Some(q) = rec.question.as_ref().filter(|q| q.answer.is_none()) {
            out.push(EventKey {
                dedupe: format!("{at}|waiting|{}|{}", q.id, q.asked_at.clone().unwrap_or_default()),
                kind: "waiting",
                detail: "one question".to_string(),
                at: q.asked_at.clone().unwrap_or_else(|| rec.updated_at.clone()),
            });
        } else if let Some(a) = rec.approval.as_ref().filter(|a| a.status == "pending") {
            out.push(EventKey {
                dedupe: format!("{at}|waiting|approval|{}", a.digest.clone().unwrap_or_default()),
                kind: "waiting",
                detail: "plan awaiting approval".to_string(),
                at: log_time(&rec.log, &["plan", "approve"], None).unwrap_or_else(|| rec.updated_at.clone()),
            });
        }
    }
    let verdict = |rec: &TaskRecord| {
        rec.verdict.as_ref().map(|v| format!("{} of {} assertions passed", v.items.iter().filter(|i| i.pass).count(), v.items.len()))
    };
    match rec.state {
        State::Succeeded => out.push(EventKey {
            dedupe: format!("{at}|done"),
            kind: "done",
            detail: verdict(rec).unwrap_or_else(|| "finished".into()),
            at: rec.verdict.as_ref().map(|v| v.at.clone()).unwrap_or_else(|| rec.updated_at.clone()),
        }),
        State::Failed => {
            let gave_up = rec.log.lines().rev().find_map(parse_failed_line);
            let detail = if rec.log.contains("cancelled by user") {
                "cancelled by user".to_string()
            } else if let Some((_, msg)) = &gave_up {
                msg.clone()
            } else {
                verdict(rec).unwrap_or_else(|| "failed".into())
            };
            let when = rec
                .verdict
                .as_ref()
                .map(|v| v.at.clone())
                .or_else(|| gave_up.map(|(at, _)| at))
                .or_else(|| log_time(&rec.log, &["execute"], Some("cancelled by user")))
                .unwrap_or_else(|| rec.updated_at.clone());
            out.push(EventKey { dedupe: format!("{at}|failed"), kind: "failed", detail, at: when });
        }
        _ => {}
    }
    out
}

/// A path that cannot be canonicalized stays in scope. A resolved path is in
/// scope only when it lives under the already-canonical search root.
fn in_root(root_canon: &Path, path: &Path) -> bool {
    match path.canonicalize() {
        Ok(canon) => canon.starts_with(root_canon),
        Err(_) => true,
    }
}

fn parse_failed_line(line: &str) -> Option<(String, String)> {
    let body = line.strip_prefix("- ")?;
    let (at, rest) = body.split_once(' ')?;
    let (_, msg) = rest.split_once(": ")?;
    msg.starts_with("failed after ").then(|| (at.to_string(), clean_message(msg, 160)))
}

#[cfg(test)]
mod tests {
    //! Containment unit tests for task 2.2
    //! (agent-workspace-scope-and-notice-routing).
    //!
    //! `records` keeps only the paths for which [`in_root`] is true, and
    //! `warn_out_of_root` warns on exactly the paths for which [`in_root`] is
    //! false. Both read the single [`in_root`] predicate, so these tests pin
    //! that predicate (and the `records`-style filter built from it) against a
    //! `tempfile` tree — mirroring `fabr-core`'s `fabric.rs` tests — covering an
    //! out-of-root path, a deeply-nested in-root path, a symlink escape, and an
    //! un-canonicalizable path. The behavioral end-to-end driving path is pinned
    //! separately by `tests/out_of_root_driven.rs`.
    //!
    //! _Requirements: 1.2, 3.1, 3.5_ · _Validates: Property 1a_

    use super::in_root;
    use std::path::{Path, PathBuf};

    /// A `tempfile` tree with the search root, a deeply-nested in-root record, a
    /// sibling directory outside the canonical root, and a symlink inside the
    /// root whose target escapes to that sibling. Mirrors the layout the broad
    /// `default_folder` search root sees in production.
    ///
    /// ```text
    /// <tmp>/
    ///   root/                                     (the canonical search root)
    ///     a/b/c/.fabric-space/tasks/deep.md       (legitimately nested, any depth)
    ///     link.md -> <tmp>/outside/escape.md      (symlink escaping the root)
    ///   outside/
    ///     escape.md                               (the real escape target)
    /// ```
    struct Tree {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        root_canon: PathBuf,
        deep: PathBuf,
        escape_link: PathBuf,
    }

    fn tree() -> Tree {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");

        // A legitimately-nested in-root record at depth (regression 3.5: a
        // search root holds records at any depth).
        let deep = root.join("a").join("b").join("c").join(".fabric-space").join("tasks").join("deep.md");
        std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
        std::fs::write(&deep, "deep").unwrap();

        // A real file under the sibling `outside` dir, and a symlink inside the
        // root pointing at it: the link is lexically in-root but canonicalizes
        // outside — a genuine search-root escape.
        std::fs::create_dir_all(&outside).unwrap();
        let escape_target = outside.join("escape.md");
        std::fs::write(&escape_target, "escape").unwrap();
        let escape_link = root.join("link.md");
        symlink_file(&escape_target, &escape_link);

        // `self.root` is canonicalized once by both callers; mirror that here.
        let root_canon = root.canonicalize().unwrap();
        Tree { _tmp: tmp, root, root_canon, deep, escape_link }
    }

    fn symlink_file(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, link).unwrap();
    }

    /// The `records`-style filter: keep exactly the paths `records` keeps
    /// (`in_root` true). Built from the same predicate `records` applies so the
    /// test exercises the real containment rule rather than a copy of it.
    fn kept(root_canon: &Path, listed: &[PathBuf]) -> Vec<PathBuf> {
        listed.iter().filter(|p| in_root(root_canon, p)).cloned().collect()
    }

    #[test]
    fn nested_in_root_path_is_kept() {
        let t = tree();
        assert!(
            in_root(&t.root_canon, &t.deep),
            "a legitimately-nested in-root record must be kept (regression 3.5): {}",
            t.deep.display()
        );
    }

    #[test]
    fn symlink_escape_path_is_excluded() {
        let t = tree();
        // Sanity: the escape link is lexically inside the (un-canonicalized)
        // root, yet canonicalizes outside — the exact production escape.
        assert!(t.escape_link.starts_with(&t.root), "the escape link must be lexically in-root: {}", t.escape_link.display());
        assert!(
            !t.escape_link.canonicalize().unwrap().starts_with(&t.root_canon),
            "the escape link must canonicalize outside the root: {}",
            t.escape_link.display()
        );
        assert!(
            !in_root(&t.root_canon, &t.escape_link),
            "a symlink escaping the search root must be excluded: {}",
            t.escape_link.display()
        );
    }

    #[test]
    fn out_of_root_path_is_excluded_from_records() {
        let t = tree();
        // A real path under the sibling `outside` dir (not reached via the
        // in-root link): directly out of the canonical root.
        let out_of_root = t._tmp.path().join("outside").join("escape.md");
        let listed = [t.deep.clone(), out_of_root.clone(), t.escape_link.clone()];
        let kept = kept(&t.root_canon, &listed);
        assert_eq!(
            kept,
            vec![t.deep.clone()],
            "records must keep only the in-root record, dropping the out-of-root path and the symlink escape"
        );
        assert!(!in_root(&t.root_canon, &out_of_root), "an out-of-root path must be excluded from records: {}", out_of_root.display());
    }

    #[test]
    fn uncanonicalizable_path_is_kept_in_scope() {
        let t = tree();
        // A path that does not exist on disk cannot be canonicalized; a
        // transient stat failure must never silently drop a real in-root record,
        // so `in_root` keeps it in-scope (returns true).
        let missing = t.root.join("a").join("b").join("c").join(".fabric-space").join("tasks").join("not-on-disk.md");
        assert!(missing.canonicalize().is_err(), "the fixture path must be un-canonicalizable: {}", missing.display());
        assert!(in_root(&t.root_canon, &missing), "an un-canonicalizable path must be kept in-scope: {}", missing.display());
    }

    #[test]
    fn in_root_matches_the_verdict_warn_out_of_root_uses() {
        let t = tree();
        // `records` keeps `in_root`-true paths; `warn_out_of_root` warns on
        // `in_root`-false paths. Over the same listed set the two must partition
        // it exactly — kept and warned are disjoint and together cover every
        // path — so detection and exclusion cannot drift.
        let out_of_root = t._tmp.path().join("outside").join("escape.md");
        let listed = [t.deep.clone(), out_of_root.clone(), t.escape_link.clone()];
        let kept: Vec<_> = listed.iter().filter(|p| in_root(&t.root_canon, p)).cloned().collect();
        let warned: Vec<_> = listed.iter().filter(|p| !in_root(&t.root_canon, p)).cloned().collect();
        assert_eq!(kept, vec![t.deep.clone()], "records keeps only the in-root record");
        assert_eq!(warned, vec![out_of_root.clone(), t.escape_link.clone()], "warn_out_of_root warns on exactly the excluded records");
        assert!(kept.iter().all(|p| !warned.contains(p)), "kept and warned must be disjoint");
        assert_eq!(kept.len() + warned.len(), listed.len(), "every listed path is either kept or warned, never both or neither");
    }
}
