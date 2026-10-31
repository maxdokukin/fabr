mod common;

use common::{FakeClaude, define_out, exec_out, ok, plan_out, verdict_out};
use fabr_agent::runner::{Options, Runner, Sink};
use fabr_agent::task_path_key;
use fabr_core::Workspace;
use fabr_core::fabric::{self, FabricCreateReq};
use fabr_core::setup::Ctx;
use fabr_core::taskrec::{self, Answer, State, Step, TaskRecord};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TOKEN: &str = "123456789:AAFAKEFAKEFAKEFAKEFAKEtoken";

struct Env {
    tmp: tempfile::TempDir,
    ctx: Ctx,
    proj: PathBuf,
    fake: FakeClaude,
}

fn env(completed: bool, claude_bin: Option<&str>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let proj = home.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let space = home.join(".fabric-space");
    std::fs::create_dir_all(&space).unwrap();
    let fake = FakeClaude::new(tmp.path());
    let bin = claude_bin.map(str::to_string).unwrap_or_else(|| fake.bin.to_string_lossy().into_owned());
    std::fs::write(
        space.join("settings.toml"),
        format!(
            "default_folder = \"{}\"\nmodel = \"haiku\"\neffort = \"low\"\nclaude_bin = \"{bin}\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"model\"]\ncompleted = {completed}\n",
            proj.display()
        ),
    )
    .unwrap();
    std::fs::write(space.join("secrets.toml"), format!("telegram_bot_token = \"{TOKEN}\"\n")).unwrap();
    Env { ctx: Ctx::new(&home), tmp, proj, fake }
}

fn ready() -> Env {
    env(true, None)
}

fn runner(e: &Env) -> (Runner, Arc<Mutex<Vec<String>>>) {
    let cfg = e.tmp.path().join("cfg");
    let cache = e.tmp.path().join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let ws = Workspace::from_config(&cfg, &cache, Some(e.proj.clone())).unwrap();
    let (sink, lines) = Sink::capture();
    (Runner::new(&e.ctx, ws, Options { interval_s: 5, once: false }, sink).unwrap(), lines)
}

fn new_record(e: &Env, title: &str) -> PathBuf {
    let req = FabricCreateReq {
        folder: e.proj.to_string_lossy().into_owned(),
        title: title.into(),
        due: Some("2026-11-01".into()),
        pri: None,
        context: Some("some context".into()),
        spec: None,
        success: None,
    };
    PathBuf::from(fabric::create_in(&req, &e.ctx.home).unwrap().path)
}

fn load(p: &Path) -> TaskRecord {
    taskrec::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn edit(p: &Path, f: impl FnOnce(&mut TaskRecord)) {
    let mut rec = load(p);
    let expected = rec.updated_at.clone();
    f(&mut rec);
    taskrec::write_record(p, &rec, Some(&expected)).unwrap();
}

fn answer(p: &Path, value: &str) {
    edit(p, |rec| {
        rec.question.as_mut().unwrap().answer = Some(Answer {
            value: Some(value.into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-24T18:05:02Z".into()),
        });
    });
}

fn queue_question(e: &Env) {
    e.fake.push(ok(define_out(Some("Which filename?")), 0.012));
}

fn queue_plan(e: &Env) {
    e.fake.push(ok(define_out(None), 0.01));
    e.fake.push(ok(plan_out(&["Write"]), 0.02));
}

fn events(e: &Env) -> Vec<String> {
    std::fs::read_to_string(e.ctx.space().join("events.log")).unwrap_or_default().lines().map(String::from).collect()
}

fn registry_files(e: &Env) -> Vec<String> {
    let rd = std::fs::read_dir(e.ctx.space().join("registry"))
        .map(|rd| rd.flatten().map(|f| f.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let mut v: Vec<String> = rd;
    v.retain(|n| n != "_backend.yaml");
    v
}

fn refuses(e: &Env, needle: &str) {
    let cfg = e.tmp.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let ws = Workspace::from_config(&cfg, &e.tmp.path().join("cache"), Some(e.proj.clone())).unwrap();
    let err = Runner::new(&e.ctx, ws, Options { interval_s: 5, once: true }, Sink::default()).err().expect("gate must refuse").message();
    assert!(err.contains(needle), "{err}");
    assert_eq!(err.matches(". ").count() + err.matches('\n').count(), 0, "one sentence: {err}");
}

#[test]
fn start_gate_refuses_without_setup_or_claude_code() {
    refuses(&env(false, None), "Setup is not complete");
    refuses(&env(true, Some("/nowhere/claude")), "Claude Code was not found at /nowhere/claude");
    let e = ready();
    std::fs::write(
        e.ctx.settings_path(),
        std::fs::read_to_string(e.ctx.settings_path()).unwrap().replace(&e.proj.display().to_string(), "/no/such/dir"),
    )
    .unwrap();
    refuses(&e, "does not exist");
    let e = ready();
    std::fs::remove_file(e.ctx.settings_path()).unwrap();
    refuses(&e, "Setup has not been started");
    let e = ready();
    assert!(fabr_agent::runner::start_gate(&e.ctx).unwrap().claude_bin == e.fake.bin);
}

#[test]
fn heartbeat_is_written_and_removed() {
    let e = ready();
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    let file = e.ctx.space().join("registry/_backend.yaml");
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(doc["heartbeat_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(doc["heartbeat_interval_s"].as_u64(), Some(10));
    assert_eq!(doc["pid"].as_u64(), Some(std::process::id() as u64));
    let start = lines.lock().unwrap().join("\n");
    assert!(start.contains("default_folder=") && start.contains("model=haiku effort=low"), "{start}");
    assert!(start.contains(&format!("claude={}", e.fake.bin.display())) && !start.contains("key="), "{start}");
    r.shutdown(true);
    assert!(!file.exists());
    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.shutdown(false);
    assert!(file.exists());
    assert_eq!(e.fake.calls(), 0);
}

#[test]
fn stale_registry_entries_are_removed_at_start() {
    let e = ready();
    let reg = e.ctx.space().join("registry");
    std::fs::create_dir_all(&reg).unwrap();
    let entry =
        |at: &str, path: &str| format!("task_path: {path}\ntask_id: t\nheartbeat_at: \"{at}\"\nheartbeat_interval_s: 10\npid: 999999\n");
    let now = format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"));
    std::fs::write(reg.join("dead--aaaaaaaa.yaml"), entry("2020-01-01T00:00:00Z", "/x/.fabric-space/tasks/dead.md")).unwrap();
    std::fs::write(reg.join("live--bbbbbbbb.yaml"), entry(&now, "/x/.fabric-space/tasks/live.md")).unwrap();
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    r.shutdown(true);
    assert_eq!(registry_files(&e), ["live--bbbbbbbb.yaml"]);
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("removing stale registry entry") && l.contains("dead--aaaaaaaa.yaml")));
    let ev = events(&e);
    assert_eq!(ev.len(), 1);
    assert!(ev[0].contains(" stuck   /x/.fabric-space/tasks/dead.md "), "{}", ev[0]);
}

#[test]
fn delegate_advances_to_a_question_and_emits_waiting_once() {
    let e = ready();
    queue_question(&e);
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert!(!rec.requests.delegate);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(rec.waiting && rec.question.as_ref().unwrap().text == "Which filename?");
    assert!((rec.cost_usd - 0.012).abs() < 1e-9, "{}", rec.cost_usd);
    assert!(rec.sessions.plan.as_deref().is_some_and(|s| s.len() == 36));
    r.tick();
    r.tick();
    let ev = events(&e);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert!(ev[0].contains(" waiting ") && ev[0].contains(&task_path_key(&path)) && ev[0].ends_with("one question"), "{}", ev[0]);
    assert_eq!(e.fake.calls(), 1);
    assert_eq!(e.fake.cwd(1), e.proj.canonicalize().unwrap());
    assert_eq!(e.fake.flag(1, "--session-id"), rec.sessions.plan);
    assert!(e.fake.stdin(1).contains("Task title: write a greeting") && e.fake.stdin(1).contains("some context"));
    let calls: Vec<String> = lines.lock().unwrap().iter().filter(|l| l.starts_with("claude run")).cloned().collect();
    assert_eq!(calls.len(), 1);
    for part in [&format!("record={}", rec.id), "step=define", "model=haiku", "turns=2", "in=120", "out=40", "cost_usd=0.012000"] {
        assert!(calls[0].contains(part), "{part} missing from {}", calls[0]);
    }
    r.shutdown(true);
}

#[test]
fn answered_question_is_copied_to_answers_and_the_pipeline_advances() {
    let e = ready();
    queue_question(&e);
    queue_plan(&e);
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    answer(&path, "greet.md");
    r.tick();
    let rec = load(&path);
    assert!(rec.question.is_none());
    assert_eq!(rec.answers[0].answer.as_ref().unwrap().value.as_deref(), Some("greet.md"));
    assert!(rec.log.contains("define: answered q1"), "{}", rec.log);
    assert_eq!((rec.state, rec.waiting, rec.approval.as_ref().unwrap().status.as_str()), (State::Defined, true, "pending"));
    let sid = rec.sessions.plan.clone();
    assert_eq!((e.fake.flag(2, "--resume"), e.fake.flag(3, "--resume")), (sid.clone(), sid));
    assert!(e.fake.stdin(2).contains("User answered: greet.md"), "{}", e.fake.stdin(2));
    let ev = events(&e);
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(ev[1].contains(" waiting ") && ev[1].ends_with("plan awaiting approval"));
    r.shutdown(true);
}

#[test]
fn cancel_on_in_progress_ends_failed_without_a_registry_entry() {
    let e = ready();
    let path = new_record(&e, "long job");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.requests.cancel = true;
    });
    let rec = load(&path);
    fabr_agent::lifecycle::write_registry(&e.ctx.space(), &e.proj, &path, &rec, 5).unwrap();
    assert_eq!(registry_files(&e).len(), 1);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!(rec.state, State::Failed);
    assert!(!rec.requests.cancel && rec.verdict.is_none() && rec.log.contains("cancelled by user"));
    assert!(registry_files(&e).is_empty());
    assert_eq!(e.fake.calls(), 0);
    assert!(events(&e).iter().any(|l| l.contains(" failed  ") && l.ends_with("cancelled by user")));
    r.shutdown(true);
}

#[test]
fn redelegate_creates_the_linked_successor() {
    let e = ready();
    queue_question(&e);
    let old = new_record(&e, "retry me");
    edit(&old, |r| {
        r.state = State::Failed;
        r.requests.redelegate = true;
        r.sessions.execute = Some("old-session".into());
    });
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let old_rec = load(&old);
    assert!(!old_rec.requests.redelegate);
    let to = old_rec.links.redelegated_to.clone().expect("old links to successor");
    let new = load(Path::new(&to));
    assert_eq!(new.links.redelegated_from.as_deref(), Some(task_path_key(&old).as_str()));
    assert_eq!((new.title.as_str(), new.due.as_deref()), ("retry me", Some("2026-11-01")));
    assert!(new.context.contains("some context") && new.id != old_rec.id);
    assert_eq!(new.state, State::Task);
    assert!(!new.requests.delegate && new.sessions.execute.is_none());
    r.shutdown(true);
}

#[test]
fn requests_that_do_not_apply_are_ignored_and_cleared() {
    let e = ready();
    let path = new_record(&e, "nothing to cancel");
    edit(&path, |r| r.requests.cancel = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert!(!rec.requests.cancel);
    assert_eq!(rec.state, State::Reminder);
    assert!(rec.log.contains("ignored request cancel in state reminder"));
    r.shutdown(true);
}

#[test]
fn a_bot_token_planted_in_claude_output_comes_out_redacted() {
    let e = ready();
    let n = e.fake.push(vec![format!("@ERR auth failed for {TOKEN}")]);
    e.fake.exit_code(n, 1);
    let path = new_record(&e, "will fail");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    r.tick();
    r.shutdown(true);
    let all = lines.lock().unwrap().join("\n");
    assert!(all.contains("stopped at") && all.contains("[bot token]"), "{all}");
    assert!(!all.contains(TOKEN), "{all}");
    let rec = load(&path);
    assert!(rec.log.contains("define: error 1: Claude Code ended without a result (exit 1): auth failed for [bot token]"), "{}", rec.log);
    for ent in walk(e.tmp.path()) {
        let leaked = std::fs::read_to_string(&ent).map(|t| t.contains(TOKEN)).unwrap_or(false);
        assert!(!leaked || ent.ends_with("secrets.toml") || ent.starts_with(&e.fake.dir), "{}", ent.display());
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(dir).unwrap().flatten() {
        let p = ent.path();
        if p.is_dir() { out.extend(walk(&p)) } else { out.push(p) }
    }
    out
}

fn registry_doc(e: &Env) -> serde_yaml::Value {
    let files = registry_files(e);
    assert_eq!(files.len(), 1, "{files:?}");
    serde_yaml::from_str(&std::fs::read_to_string(e.ctx.space().join("registry").join(&files[0])).unwrap()).unwrap()
}

fn set_setting(e: &Env, line: &str) {
    let path = e.ctx.settings_path();
    let key = line.split(' ').next().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().filter(|l| !l.starts_with(key)).collect();
    std::fs::write(&path, format!("{line}\n{}\n", kept.join("\n"))).unwrap();
}

#[test]
fn stage_errors_are_logged_held_across_a_restart_and_end_in_failed() {
    let e = ready();
    let path = new_record(&e, "will fail");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(
        rec.log.contains("define: error 1: Claude Code ended without a result (exit 3): fake claude has no canned output"),
        "{}",
        rec.log
    );
    assert_eq!(e.fake.calls(), 1);
    let doc = registry_doc(&e);
    assert_eq!((doc["activity"].as_str(), doc["step"].as_str()), (Some("retry"), Some("define")));
    assert!(doc["retry_in_s"].as_u64().unwrap() > 0);
    assert!(doc["last_error"].as_str().unwrap().contains("exit 3"));
    r.tick();
    assert_eq!(e.fake.calls(), 1);
    r.shutdown(true);

    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.tick();
    assert_eq!(e.fake.calls(), 1, "the backoff must survive a restart");
    again.shutdown(true);

    let (eager, _) = runner(&e);
    let mut eager = eager.with_retry_base_s(0);
    eager.start().unwrap();
    eager.tick();
    assert!(load(&path).log.contains("define: error 2: "));
    eager.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Failed, None));
    assert!(rec.verdict.is_none() && !rec.waiting);
    assert!(rec.log.contains("define: failed after 3 errors"), "{}", rec.log);
    assert!(registry_files(&e).is_empty());
    assert!(events(&e).iter().any(|l| l.contains(" failed  ") && l.contains("failed after 3 errors")), "{:?}", events(&e));
    eager.tick();
    assert_eq!(e.fake.calls(), 3);
    eager.shutdown(true);
}

#[test]
fn an_error_result_or_missing_structured_output_is_a_stage_error() {
    let e = ready();
    e.fake.push(vec![
        common::init(),
        json!({"type": "result", "subtype": "error_max_turns", "is_error": true, "session_id": "@SID@", "total_cost_usd": 0.5, "result": ""}).to_string(),
    ]);
    e.fake.push(vec![
        common::init(),
        json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "@SID@", "total_cost_usd": 0.25, "result": "hi"})
            .to_string(),
    ]);
    e.fake.push(vec![
        common::init(),
        json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "@SID@", "total_cost_usd": 0.1, "result": "still prose"})
            .to_string(),
    ]);
    let path = new_record(&e, "errors");
    edit(&path, |r| r.requests.delegate = true);
    let (r, _) = runner(&e);
    let mut r = r.with_retry_base_s(0);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert!(rec.log.contains("define: error 1: Claude Code reported error_max_turns"), "{}", rec.log);
    r.tick();
    let rec = load(&path);
    assert!(rec.log.contains("define: error 2: Claude Code returned no structured output: hi"), "{}", rec.log);
    assert!(e.fake.stdin(3).contains("calling the StructuredOutput tool"), "{}", e.fake.stdin(3));
    assert_eq!(e.fake.flag(3, "--resume"), rec.sessions.plan.clone());
    assert!((rec.cost_usd - 0.85).abs() < 1e-9, "{}", rec.cost_usd);
    assert_eq!(e.fake.flag(2, "--resume"), rec.sessions.plan);
    r.shutdown(true);
}

#[test]
fn the_registry_entry_follows_the_task_from_gather_to_notify() {
    let e = ready();
    queue_question(&e);
    queue_plan(&e);
    e.fake.push(vec![
        common::init(),
        common::tool_use("Write", json!({"file_path": "greet.md", "content": "hi"})),
        "@SH printf hi > greet.md".into(),
        common::result(exec_out("done", "wrote greet.md"), 0.05, json!([])),
    ]);
    e.fake.push(ok(verdict_out(true), 0.01));
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let doc = registry_doc(&e);
    assert_eq!((doc["state"].as_str(), doc["step"].as_str()), (Some("task"), Some("define")));
    assert_eq!((doc["waiting"].as_str(), doc["activity"].as_str()), (Some("question"), Some("parked")));
    assert!(doc["retry_in_s"].is_null() && doc["last_error"].is_null());
    assert_eq!(doc["usage"]["define"]["calls"].as_u64(), Some(1));
    assert_eq!(doc["usage"]["define"]["input_tokens"].as_u64(), Some(120));
    assert_eq!(doc["usage"]["define"]["output_tokens"].as_u64(), Some(40));
    assert!((doc["usage"]["define"]["usd"].as_f64().unwrap() - 0.012).abs() < 1e-9);
    assert_eq!(doc["question"]["text"].as_str(), Some("Which filename?"));
    let started = doc["started_at"].as_str().unwrap().to_string();

    answer(&path, "greet.md");
    r.tick();
    let doc = registry_doc(&e);
    assert_eq!(
        (doc["state"].as_str(), doc["step"].as_str(), doc["waiting"].as_str()),
        (Some("defined"), Some("approve"), Some("approval"))
    );
    assert_eq!(doc["started_at"].as_str(), Some(started.as_str()));
    assert_eq!((doc["usage"]["plan"]["calls"].as_u64(), doc["usage"]["define"]["calls"].as_u64()), (Some(1), Some(2)));
    let cost = doc["cost_usd"].as_f64().unwrap();
    assert!((cost - load(&path).cost_usd).abs() < 1e-9 && (cost - 0.042).abs() < 1e-9, "{cost}");

    edit(&path, |rec| {
        let a = rec.approval.as_mut().unwrap();
        a.status = "approved".into();
        a.by = Some("gui".into());
    });
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None), "{}", rec.log);
    assert!(rec.log.contains("execute: Write greet.md"), "{}", rec.log);
    assert!((rec.cost_usd - 0.102).abs() < 1e-9, "{}", rec.cost_usd);
    assert!(registry_files(&e).is_empty());
    r.shutdown(true);
}

#[test]
fn the_heartbeat_interval_comes_from_settings() {
    let e = ready();
    queue_question(&e);
    set_setting(&e, "heartbeat_interval_s = 3");
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(e.ctx.space().join("registry/_backend.yaml")).unwrap()).unwrap();
    assert_eq!(doc["heartbeat_interval_s"].as_u64(), Some(3));
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    r.tick();
    assert_eq!(registry_doc(&e)["heartbeat_interval_s"].as_u64(), Some(3));
    r.shutdown(true);
}

fn finished(e: &Env, title: &str, at: &str) -> PathBuf {
    let path = new_record(e, title);
    edit(&path, |r| {
        r.state = State::Succeeded;
        r.step = None;
        r.verdict = Some(taskrec::Verdict { result: "pass".into(), at: at.into(), items: Vec::new() });
    });
    path
}

fn now_ts() -> String {
    format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
}

#[test]
fn events_missed_while_the_runner_was_down_are_emitted_once_after_restart() {
    let e = ready();
    let space = e.ctx.space();
    std::fs::write(space.join("events.log"), "2026-01-01T00:00:00Z done    /elsewhere/.fabric-space/tasks/x.md 1 of 1 assertions passed\n")
        .unwrap();
    let old = finished(&e, "finished long ago", "2025-12-31T00:00:00Z");
    let missed = finished(&e, "finished while down", &now_ts());
    let announced = finished(&e, "already announced", &now_ts());
    let asking = new_record(&e, "asking");
    edit(&asking, |r| {
        r.state = State::Task;
        r.step = Some(Step::Define);
        r.waiting = true;
        r.question = Some(taskrec::Question {
            id: "q1".into(),
            kind: "open".into(),
            text: "Which?".into(),
            options: None,
            prior: None,
            fields: None,
            asked_at: Some(now_ts()),
            answer: None,
        });
    });
    let mut log = std::fs::read_to_string(space.join("events.log")).unwrap();
    log.push_str(&format!("{} done    {} 0 of 0 assertions passed\n", now_ts(), task_path_key(&announced)));
    std::fs::write(space.join("events.log"), log).unwrap();

    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    r.tick();
    r.shutdown(true);
    let ev = events(&e);
    let count = |p: &Path, kind: &str| ev.iter().filter(|l| l.contains(&format!(" {kind}")) && l.contains(&task_path_key(p))).count();
    assert_eq!(count(&missed, "done"), 1, "{ev:?}");
    assert_eq!(count(&announced, "done"), 1, "{ev:?}");
    assert_eq!(count(&old, "done"), 0, "{ev:?}");
    assert_eq!(count(&asking, "waiting"), 1, "{ev:?}");

    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.tick();
    again.shutdown(true);
    assert_eq!(events(&e).len(), ev.len(), "a second restart must not repeat events");
    assert_eq!(e.fake.calls(), 0);
}

fn to_awaiting_approval(e: &Env, r: &mut Runner, title: &str) -> PathBuf {
    queue_plan(e);
    let path = new_record(e, title);
    edit(&path, |rec| rec.requests.delegate = true);
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)), "{}", rec.log);
    path
}

fn approve(path: &Path) {
    edit(path, |rec| {
        let a = rec.approval.as_mut().unwrap();
        a.status = "approved".into();
        a.by = Some("gui".into());
    });
}

#[test]
fn pause_keeps_the_step_and_the_registry_says_paused_until_resume() {
    let e = ready();
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let path = to_awaiting_approval(&e, &mut r, "pause me");
    let calls = e.fake.calls();
    edit(&path, |rec| rec.requests.pause = true);
    r.tick();
    let rec = load(&path);
    assert!(rec.paused && !rec.requests.pause && rec.waiting);
    assert!(rec.log.contains("approve: paused by user"), "{}", rec.log);
    let doc = registry_doc(&e);
    assert_eq!(
        (doc["activity"].as_str(), doc["waiting"].as_str(), doc["step"].as_str()),
        (Some("paused"), Some("approval"), Some("approve"))
    );
    approve(&path);
    r.tick();
    r.tick();
    assert_eq!((load(&path).state, e.fake.calls()), (State::Defined, calls));
    r.shutdown(true);
    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.tick();
    assert_eq!(registry_doc(&e)["activity"].as_str(), Some("paused"));
    edit(&path, |rec| rec.requests.resume = true);
    again.tick();
    let rec = load(&path);
    assert!(!rec.paused && !rec.requests.resume);
    assert!(rec.log.contains("approve: resumed by user"), "{}", rec.log);
    assert_ne!(rec.state, State::Defined);
    again.shutdown(true);
}

#[test]
fn resume_without_pause_and_pause_on_a_reminder_are_ignored() {
    let e = ready();
    let path = new_record(&e, "idle");
    edit(&path, |rec| {
        rec.requests.pause = true;
        rec.requests.resume = true;
    });
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert!(!rec.paused && !rec.requests.pause && !rec.requests.resume);
    assert!(rec.log.contains("ignored request pause in state reminder") && rec.log.contains("ignored request resume in state reminder"));
    r.shutdown(true);
}

#[test]
fn suspend_removes_the_entry_logs_once_and_skips_the_record() {
    let e = ready();
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let reminder = new_record(&e, "shelved reminder");
    edit(&reminder, |rec| {
        rec.requests.delegate = true;
        rec.suspended = true;
    });
    let path = to_awaiting_approval(&e, &mut r, "shelve me");
    assert_eq!(registry_files(&e).len(), 1);
    assert_eq!(load(&reminder).state, State::Reminder);
    edit(&path, |rec| rec.suspended = true);
    approve(&path);
    for _ in 0..3 {
        r.tick();
    }
    let rec = load(&path);
    assert_eq!(rec.state, State::Defined);
    assert_eq!(rec.log.matches("approve: suspended").count(), 1, "{}", rec.log);
    assert!(registry_files(&e).is_empty());
    let shelved = load(&reminder);
    assert!(shelved.state == State::Reminder && shelved.requests.delegate);
    r.shutdown(true);
}

#[test]
fn a_passed_suspend_until_clears_itself_and_driving_resumes() {
    let e = ready();
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let path = to_awaiting_approval(&e, &mut r, "back soon");
    edit(&path, |rec| {
        rec.suspended = true;
        rec.suspend_until = Some("2999-01-01".into());
    });
    r.tick();
    assert!(registry_files(&e).is_empty());
    edit(&path, |rec| rec.suspend_until = Some("2000-01-01T00:00".into()));
    r.tick();
    let rec = load(&path);
    assert!(!rec.suspended && rec.suspend_until.is_none());
    assert!(rec.log.contains(" unsuspended: timer"), "{}", rec.log);
    assert_eq!(registry_files(&e).len(), 1);
    r.shutdown(true);
}

#[test]
fn a_paused_task_does_not_block_the_next_one_in_the_folder() {
    let e = ready();
    let blocker = new_record(&e, "paused job");
    edit(&blocker, |rec| {
        rec.state = State::InProgress;
        rec.step = Some(Step::Execute);
        rec.paused = true;
    });
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let next = to_awaiting_approval(&e, &mut r, "next job");
    approve(&next);
    r.tick();
    assert_ne!(load(&next).state, State::Defined);
    let b = load(&blocker);
    assert!(b.paused && b.state == State::InProgress);
    r.shutdown(true);
}

#[test]
fn complete_during_execute_succeeds_and_emits_done() {
    let e = ready();
    let path = new_record(&e, "long job");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.requests.complete = true;
    });
    let rec = load(&path);
    fabr_agent::lifecycle::write_registry(&e.ctx.space(), &e.proj, &path, &rec, 5).unwrap();
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None));
    assert!(!rec.requests.complete && !rec.paused && rec.verdict.is_none());
    assert!(rec.log.contains("execute: completed by user"), "{}", rec.log);
    assert!(registry_files(&e).is_empty());
    assert_eq!(e.fake.calls(), 0);
    assert!(events(&e).iter().any(|l| l.contains(" done ")), "{:?}", events(&e));
    r.shutdown(true);
}

#[test]
fn complete_while_paused_succeeds() {
    let e = ready();
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let path = to_awaiting_approval(&e, &mut r, "pause then done");
    edit(&path, |rec| rec.requests.pause = true);
    r.tick();
    assert!(load(&path).paused);
    edit(&path, |rec| rec.requests.complete = true);
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None));
    assert!(!rec.paused && !rec.requests.complete);
    assert!(rec.log.contains("approve: completed by user"), "{}", rec.log);
    assert!(registry_files(&e).is_empty());
    r.shutdown(true);
}

#[test]
fn complete_on_a_reminder_goes_through_the_runner() {
    let e = ready();
    let path = new_record(&e, "by hand");
    edit(&path, |rec| rec.requests.complete = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded);
    assert!(!rec.requests.complete);
    assert!(rec.log.contains("done: completed by user"), "{}", rec.log);
    r.shutdown(true);
}

#[test]
fn the_whole_pipeline_runs_through_claude_code_sessions() {
    let e = ready();
    std::fs::write(e.proj.join("notes.md"), "say hi").unwrap();
    let notes = e.proj.join("notes.md").to_string_lossy().into_owned();
    queue_question(&e);
    e.fake.push(ok(define_out(None), 0.01));
    e.fake.push(ok(plan_out(&["Write"]), 0.02));
    e.fake.push(vec![
        common::init(),
        common::tool_use("Read", json!({"file_path": notes})),
        common::tool_use("Bash", json!({"command": "npm install left-pad"})),
        common::result(
            exec_out("blocked", "npm install was denied"),
            0.03,
            json!([{"tool_name": "Bash", "tool_use_id": "t", "tool_input": {"command": "npm install left-pad"}}]),
        ),
    ]);
    e.fake.push(vec![
        common::init(),
        common::tool_use("Bash", json!({"command": "npm install left-pad"})),
        common::tool_use("Write", json!({"file_path": "greet.md", "content": "hi"})),
        "@SH printf hi > greet.md".into(),
        common::result(exec_out("done", "installed left-pad and wrote greet.md"), 0.04, json!([])),
    ]);
    e.fake.push(ok(verdict_out(true), 0.005));
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();

    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step, rec.waiting), (State::Task, Some(Step::Define), true));
    assert!(rec.context.contains("### Gathered") && rec.context.contains("notes.md: say hi"), "{}", rec.context);
    let planner = rec.sessions.plan.clone().unwrap();
    assert_eq!(e.fake.flag(1, "--session-id").as_deref(), Some(planner.as_str()));
    for (flag, want) in [
        ("--permission-mode", "plan"),
        ("--permission-prompts", "none"),
        ("--tools", "Read,Glob,Grep"),
        ("--allowedTools", "Read Glob Grep"),
        ("--output-format", "stream-json"),
        ("--model", "haiku"),
        ("--effort", "low"),
    ] {
        assert_eq!(e.fake.flag(1, flag).as_deref(), Some(want), "{flag}");
    }
    assert!(e.fake.has(1, "-p") && e.fake.has(1, "--verbose"));
    assert!(e.fake.flag(1, "--json-schema").unwrap().contains("assertions"));

    answer(&path, "greet.md");
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert!(rec.plan.contains("### Outputs"));
    for n in [2, 3] {
        assert_eq!(e.fake.flag(n, "--resume").as_deref(), Some(planner.as_str()), "call {n}");
        assert!(!e.fake.has(n, "--session-id"));
    }
    assert!(e.fake.flag(3, "--json-schema").unwrap().contains("reversibility"));
    assert!(e.fake.stdin(3).contains("### Inputs") && e.fake.stdin(3).contains("Bash(npm test:*)"));

    approve(&path);
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step, rec.waiting), (State::InProgress, Some(Step::Execute), true));
    let executor = rec.sessions.execute.clone().unwrap();
    assert_ne!(executor, planner);
    assert_eq!(e.fake.flag(4, "--session-id").as_deref(), Some(executor.as_str()));
    assert_eq!(e.fake.flag(4, "--permission-mode").as_deref(), Some("acceptEdits"));
    assert_eq!(e.fake.flag(4, "--permission-prompts").as_deref(), Some("none"));
    assert_eq!(e.fake.flag(4, "--allowedTools").as_deref(), Some("Read Glob Grep Write"));
    assert!(e.fake.flag(4, "--disallowedTools").unwrap().starts_with("Edit(./.fabric-space/**) Edit(.fabric-space/**) Write("));
    assert!(!e.fake.has(1, "--disallowedTools"));
    assert!(!e.fake.has(4, "--tools"));
    assert!(e.fake.stdin(4).contains("Approved plan:") && e.fake.stdin(4).contains("### Steps"));
    for line in
        ["execute: Read notes.md", "execute: Bash npm install left-pad", "execute: denied Bash(npm install left-pad)", "execute: asked q2"]
    {
        assert!(rec.log.contains(line), "{line}: {}", rec.log);
    }
    let q = rec.question.as_ref().unwrap();
    assert!(
        q.text.contains("run `npm install left-pad`") && q.text.contains("Reply allow to grant `Bash(npm install left-pad)`"),
        "{}",
        q.text
    );
    assert_eq!(registry_doc(&e)["waiting"].as_str(), Some("question"));

    answer(&path, "allow");
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None), "{}", rec.log);
    assert_eq!(e.fake.flag(5, "--resume").as_deref(), Some(executor.as_str()));
    assert_eq!(e.fake.flag(5, "--allowedTools").as_deref(), Some("Read Glob Grep Write Bash(npm install left-pad)"));
    assert!(e.fake.stdin(5).contains("Continue the task") && e.fake.stdin(5).contains("User answered: allow"));
    assert!(rec.log.contains("execute: allowed Bash(npm install left-pad) by user for q2; approval re-signed"), "{}", rec.log);
    assert_eq!(rec.plan_meta.as_ref().unwrap().tools, ["Write", "Bash(npm install left-pad)"]);
    assert_eq!(rec.approval.as_ref().unwrap().digest.as_deref(), Some(taskrec::approval_digest(&rec).as_str()));
    assert_eq!(std::fs::read_to_string(e.proj.join("greet.md")).unwrap(), "hi");
    let validator = rec.sessions.evaluate.clone().unwrap();
    assert!(validator != executor && validator != planner);
    assert_eq!(e.fake.flag(6, "--session-id").as_deref(), Some(validator.as_str()));
    assert_eq!(e.fake.flag(6, "--permission-mode").as_deref(), Some("plan"));
    assert_eq!(e.fake.flag(6, "--tools").as_deref(), Some("Read,Glob,Grep"));
    assert!(e.fake.stdin(6).contains("installed left-pad and wrote greet.md") && e.fake.stdin(6).contains("test -f greet.md exit 0"));
    let v = rec.verdict.as_ref().unwrap();
    assert_eq!((v.result.as_str(), v.items[0].evidence.as_str()), ("pass", "greet.md holds hi"));
    assert!((rec.cost_usd - 0.117).abs() < 1e-9, "{}", rec.cost_usd);
    assert_eq!(e.fake.calls(), 6);
    assert!(events(&e).iter().any(|l| l.contains(" done ") && l.ends_with("1 of 1 assertions passed")), "{:?}", events(&e));
    assert!(registry_files(&e).is_empty());
    assert!(rec.log.contains("learn: captured 1 answer"), "{}", rec.log);
    let learned = fabr_core::knowledge::read_learned(&e.proj).unwrap();
    assert_eq!(learned.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["greet.md"]);
    r.shutdown(true);
}

#[test]
fn a_run_that_hits_the_spend_cap_fails_the_task_without_retries() {
    let e = ready();
    let settings = e.ctx.settings_path();
    let text = std::fs::read_to_string(&settings).unwrap();
    std::fs::write(&settings, format!("max_budget_usd = 0.5\n{text}")).unwrap();
    e.fake.push(vec![
        common::init(),
        json!({"type": "result", "subtype": "error_max_budget_usd", "is_error": true, "session_id": "@SID@", "total_cost_usd": 0.52, "result": ""})
            .to_string(),
    ]);
    let path = new_record(&e, "capped");
    edit(&path, |r| r.requests.delegate = true);
    let (r, _) = runner(&e);
    let mut r = r.with_retry_base_s(0);
    r.start().unwrap();
    r.tick();
    r.tick();
    let rec = load(&path);
    assert_eq!(e.fake.flag(1, "--max-budget-usd").as_deref(), Some("0.50"));
    assert_eq!(rec.state, State::Failed, "{}", rec.log);
    assert!(rec.log.contains("define: stopped: spend cap reached: Claude Code stopped after $0.52"), "{}", rec.log);
    assert_eq!(e.fake.calls(), 1);
    assert!((rec.cost_usd - 0.52).abs() < 1e-9, "{}", rec.cost_usd);
    r.shutdown(true);
}

#[test]
fn start_kills_process_groups_left_by_a_dead_runner() {
    use std::os::unix::process::CommandExt;
    let e = ready();
    let mut orphan = std::process::Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
    let file = fabr_agent::runner::runs_file(&e.ctx.space());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, format!("{} 2147483000\n2147483001 2147483000\n", orphan.id())).unwrap();
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while orphan.try_wait().unwrap().is_none() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(orphan.try_wait().unwrap().is_some(), "the orphan still runs");
    let logged = lines.lock().unwrap().join("\n");
    assert!(logged.contains(&format!("killed leftover Claude Code process group {} from runner 2147483000", orphan.id())), "{logged}");
    assert!(!logged.contains("2147483001"), "{logged}");
    assert!(!file.exists());
    r.shutdown(true);
}

fn cites(id: &str, question: &str) -> serde_json::Value {
    json!({"question": null, "spec": "", "success": "", "assertions": [], "prior_id": id, "prior_question": question})
}

fn defines_with(id: &str) -> serde_json::Value {
    let mut v = define_out(None);
    v["prior_id"] = json!(id);
    v
}

fn reply(p: &Path, value: &str, replacement: Option<&str>) {
    edit(p, |rec| {
        rec.question.as_mut().unwrap().answer = Some(Answer {
            value: Some(value.into()),
            replacement: replacement.map(String::from),
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-31T10:00:00Z".into()),
        });
    });
}

#[test]
fn a_prior_is_asked_then_always_then_assumed_and_a_replacement_is_learned() {
    let e = ready();
    let agents = e.proj.join(".fabric-space/.agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("learned.yaml"),
        "records:\n  - {id: l1, text: Ship to home, applies_when: Where to ship?, source_tasks: [/p/old.md], last_confirmed: 2026-10-01T00:00:00Z}\n  - {id: l2, text: Use the blue box, applies_when: Which box?}\n",
    )
    .unwrap();
    e.fake.push(ok(cites("l1", "Ship to home again?"), 0.01));
    queue_plan(&e);
    let (mut r, _) = runner(&e);
    r.start().unwrap();

    let first = new_record(&e, "order filament");
    edit(&first, |rec| rec.requests.delegate = true);
    r.tick();
    let rec = load(&first);
    assert!(e.fake.stdin(1).contains("- l1: when Where to ship?: Ship to home"), "{}", e.fake.stdin(1));
    assert!(e.fake.flag(1, "--json-schema").unwrap().contains("prior_id"));
    let q = rec.question.as_ref().unwrap();
    assert_eq!((q.kind.as_str(), q.text.as_str(), rec.waiting), ("choice", "Ship to home again?", true));
    assert_eq!(q.options.as_deref().unwrap(), ["once", "always", "no"]);
    let prior = q.prior.as_ref().unwrap();
    assert_eq!((prior.text.as_str(), prior.source_task.as_deref()), ("Ship to home", Some("/p/old.md")));
    assert_eq!(prior.last_confirmed.as_deref(), Some("2026-10-01T00:00:00Z"));

    reply(&first, "always", None);
    r.tick();
    let rec = load(&first);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)), "{}", rec.log);
    assert!(e.fake.stdin(2).contains("User answered: always (earlier choice: Ship to home)"), "{}", e.fake.stdin(2));
    assert!(e.fake.stdin(3).contains("Always-rule l1 applies"), "{}", e.fake.stdin(3));
    let learned = fabr_core::knowledge::read_learned(&e.proj).unwrap();
    assert!(learned[0].always && !learned[1].always && rec.log.contains("define: q1 set l1 to always"), "{}", rec.log);
    assert!(std::fs::read_to_string(agents.join("learned.yaml")).unwrap().contains("always: true"));
    assert_eq!(rec.plan_meta.as_ref().unwrap().assumptions[0].rule_id, "l1");

    e.fake.push(ok(defines_with("l1"), 0.01));
    e.fake.push(ok(plan_out(&["Write"]), 0.02));
    let second = new_record(&e, "order more filament");
    edit(&second, |rec| rec.requests.delegate = true);
    r.tick();
    let rec = load(&second);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)), "{}", rec.log);
    assert!(e.fake.stdin(4).contains("- l1 (always): when Where to ship?: Ship to home"), "{}", e.fake.stdin(4));
    assert!(rec.answers.is_empty() && rec.question.is_none() && rec.log.contains("define: assumed l1 (always)"), "{}", rec.log);
    let meta = rec.plan_meta.clone().unwrap();
    assert_eq!((meta.assumptions.len(), meta.assumptions[0].text.as_str()), (1, "Ship to home"));
    let digest = rec.approval.as_ref().unwrap().digest.clone().unwrap();
    assert_eq!(digest, taskrec::approval_digest(&rec));
    let mut bare = rec.clone();
    bare.plan_meta.as_mut().unwrap().assumptions.clear();
    assert_ne!(digest, taskrec::approval_digest(&bare));

    e.fake.push(ok(cites("l2", "Blue box again?"), 0.01));
    queue_plan(&e);
    e.fake.push(vec![
        common::init(),
        "@SH printf hi > greet.md".into(),
        common::result(exec_out("done", "wrote greet.md"), 0.01, json!([])),
    ]);
    e.fake.push(ok(verdict_out(true), 0.005));
    let third = new_record(&e, "pack the order");
    edit(&third, |rec| rec.requests.delegate = true);
    r.tick();
    assert_eq!(load(&third).question.as_ref().unwrap().kind, "choice");
    reply(&third, "no", Some("the red box"));
    r.tick();
    assert!(e.fake.stdin(7).contains("no; use instead: the red box"), "{}", e.fake.stdin(7));
    approve(&third);
    r.tick();
    let rec = load(&third);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None), "{}", rec.log);
    assert!(rec.plan_meta.as_ref().unwrap().assumptions.is_empty());
    assert!(rec.log.contains("learn: captured 1 answer"), "{}", rec.log);
    let learned = fabr_core::knowledge::read_learned(&e.proj).unwrap();
    let added = learned.last().unwrap();
    assert_eq!((added.text.as_str(), added.applies_when.as_str(), added.always), ("the red box", "Which box?", false));
    assert_eq!(learned.len(), 3);
    r.shutdown(true);
}
