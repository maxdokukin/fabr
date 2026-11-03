use fabr_agent::runner::{Options, Runner, Sink};
use fabr_agent::task_path_key;
use fabr_core::Workspace;
use fabr_core::fabric::{self, FabricCreateReq};
use fabr_core::setup::Ctx;
use fabr_core::taskrec::{self, Answer, State, Step, TaskRecord};
use serde_json::json;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa";

struct Fake {
    url: String,
    bodies: Arc<Mutex<Vec<String>>>,
    questions: Arc<AtomicUsize>,
    status: Arc<AtomicUsize>,
    script: Arc<Mutex<VecDeque<u16>>>,
    cut: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    fn new(questions: usize) -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let questions = Arc::new(AtomicUsize::new(questions));
        let status = Arc::new(AtomicUsize::new(200));
        let script = Arc::new(Mutex::new(VecDeque::<u16>::new()));
        let cut = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (b, q, s, st, sc, ct) = (bodies.clone(), questions.clone(), status.clone(), stop.clone(), script.clone(), cut.clone());
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                let Ok(Some(mut req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
                let mut body = String::new();
                std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
                b.lock().unwrap().push(body.clone());
                let code = sc.lock().unwrap().pop_front().unwrap_or(s.load(Ordering::SeqCst) as u16);
                let text = if code != 200 {
                    format!("bad key {KEY} in x-api-key: {KEY}")
                } else if body.contains("keys spec, success, assertions, question") {
                    let asked = q.load(Ordering::SeqCst) > 0;
                    if asked {
                        q.fetch_sub(1, Ordering::SeqCst);
                    }
                    let question = if asked { json!("Which filename?") } else { json!(null) };
                    let inner = json!({"spec": "write greet.md", "success": "greet.md exists", "assertions": [{"id": "a1", "text": "greet.md exists"}], "question": question});
                    json!({"choices": [{"message": {"role": "assistant", "content": inner.to_string()}}], "usage": {"prompt_tokens": 120, "completion_tokens": 45}})
                        .to_string()
                } else {
                    let inner = json!({"spec": "write greet.md", "success": "greet.md exists", "plan": "1. write it", "assertions": [{"id": "a1", "text": "greet.md exists"}],
                        "plan_meta": {"reversibility": "reversible", "tools": ["write_file"], "checks": [{"cmd": "test", "args": ["-f", "greet.md"]}]}});
                    json!({"choices": [{"message": {"role": "assistant", "content": inner.to_string()}}], "usage": {"prompt_tokens": 300, "completion_tokens": 80}})
                        .to_string()
                };
                let text = if ct.load(Ordering::SeqCst) && code == 200 {
                    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    v["choices"][0]["finish_reason"] = json!("length");
                    v.to_string()
                } else {
                    text
                };
                let resp = tiny_http::Response::from_string(text)
                    .with_status_code(code)
                    .with_header(tiny_http::Header::from_bytes("content-type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        });
        Fake { url, bodies, questions, status, script, cut, stop, handle: Some(handle) }
    }

    fn calls(&self) -> usize {
        self.bodies.lock().unwrap().len()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct Env {
    tmp: tempfile::TempDir,
    ctx: Ctx,
    proj: PathBuf,
}

fn env(url: &str, completed: bool, key: Option<&str>, model: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let proj = home.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let space = home.join(".fabric-space");
    std::fs::create_dir_all(&space).unwrap();
    std::fs::write(
        space.join("settings.toml"),
        format!(
            "default_folder = \"{}\"\nmodel = \"{model}\"\nlocal_base_url = \"{url}\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\"]\ncompleted = {completed}\n",
            proj.display()
        ),
    )
    .unwrap();
    std::fs::write(space.join("secrets.toml"), key.map(|k| format!("anthropic_api_key = \"{k}\"\n")).unwrap_or_default()).unwrap();
    let ctx = Ctx::new(&home);
    Env { tmp, ctx, proj }
}

fn runner(e: &Env) -> (Runner, Arc<Mutex<Vec<String>>>) {
    let cfg = e.tmp.path().join("cfg");
    let cache = e.tmp.path().join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let ws = Workspace::from_config(&cfg, &cache, Some(e.proj.clone())).unwrap();
    let (sink, lines) = Sink::capture();
    (Runner::new(&e.ctx, ws, Options { interval_s: 5, once: false }, sink).unwrap().with_model_retry_base(Duration::from_millis(1)), lines)
}

fn ready(fake: &Fake) -> Env {
    env(&fake.url, true, Some(KEY), "claude-sonnet-5-5")
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
fn start_gate_refuses_without_setup() {
    refuses(&env("http://127.0.0.1:1", false, Some(KEY), "claude-sonnet-5-5"), "Setup is not complete");
    refuses(&env("http://127.0.0.1:1", true, Some(KEY), ""), "model");
    let e_bad_folder = env("http://127.0.0.1:1", true, Some(KEY), "claude-sonnet-5-5");
    std::fs::remove_dir_all(&e_bad_folder.proj).unwrap();
    refuses(&e_bad_folder, "does not exist");
    let e = env("http://127.0.0.1:1", true, Some(KEY), "claude-sonnet-5-5");
    std::fs::remove_file(e.ctx.settings_path()).unwrap();
    refuses(&e, "Setup has not been started");
}

#[test]
fn heartbeat_is_written_and_removed() {
    let fake = Fake::new(0);
    let e = ready(&fake);
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    let file = e.ctx.space().join("registry/_backend.yaml");
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert!(doc["heartbeat_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(doc["heartbeat_interval_s"].as_u64(), Some(10));
    assert_eq!(doc["pid"].as_u64(), Some(std::process::id() as u64));
    assert!(doc["started_at"].as_str().unwrap().ends_with('Z'));
    let start = lines.lock().unwrap().join("\n");
    assert!(
        start.contains("default_folder=") && start.contains("model=claude-sonnet-5-5") && start.contains("key=sk-ant-…x9Qa"),
        "{start}"
    );
    r.shutdown(true);
    assert!(!file.exists());
    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.shutdown(false);
    assert!(file.exists());
}

#[test]
fn stale_registry_entries_are_removed_at_start() {
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(1);
    let e = ready(&fake);
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert!(!rec.requests.delegate);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(rec.waiting);
    assert_eq!(rec.question.as_ref().unwrap().text, "Which filename?");
    assert!(rec.cost_usd > 0.0);
    r.tick();
    r.tick();
    let ev = events(&e);
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert!(ev[0].contains(" waiting ") && ev[0].contains(&task_path_key(&path)) && ev[0].ends_with("one question"), "{}", ev[0]);
    assert_eq!(fake.calls(), 1);
    let calls: Vec<String> = lines.lock().unwrap().iter().filter(|l| l.starts_with("model call")).cloned().collect();
    assert_eq!(calls.len(), 1);
    let l = &calls[0];
    for part in [
        "at=",
        &format!("record={}", rec.id),
        "step=define",
        "model=claude-sonnet-5-5",
        "in=120",
        "out=45",
        "cost_usd=0.00069",
        "key=sk-ant-…x9Qa",
    ] {
        assert!(l.contains(part), "{part} missing from {l}");
    }
    r.shutdown(true);
    assert!(!lines.lock().unwrap().join("\n").contains(KEY));
    for ent in walk(e.tmp.path()) {
        assert!(
            !std::fs::read_to_string(&ent).map(|t| t.contains(KEY)).unwrap_or(false) || ent.ends_with("secrets.toml"),
            "{}",
            ent.display()
        );
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

#[test]
fn answered_question_is_copied_to_answers_and_the_pipeline_advances() {
    let fake = Fake::new(1);
    let e = ready(&fake);
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    edit(&path, |rec| {
        rec.question.as_mut().unwrap().answer = Some(Answer {
            value: Some("greet.md".into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-24T18:05:02Z".into()),
        });
    });
    r.tick();
    let rec = load(&path);
    assert!(rec.question.is_none());
    assert_eq!(rec.answers.len(), 1);
    assert_eq!(rec.answers[0].answer.as_ref().unwrap().value.as_deref(), Some("greet.md"));
    assert_eq!(rec.state, State::Defined);
    assert!(rec.waiting);
    assert_eq!(rec.approval.as_ref().unwrap().status, "pending");
    let ev = events(&e);
    assert_eq!(ev.len(), 2, "{ev:?}");
    assert!(ev[1].contains(" waiting ") && ev[1].ends_with("plan awaiting approval"));
    r.shutdown(true);
}

#[test]
fn cancel_on_in_progress_ends_failed_without_a_registry_entry() {
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    assert!(!rec.requests.cancel);
    assert!(rec.verdict.is_none());
    assert!(rec.log.contains("cancelled by user"));
    assert!(registry_files(&e).is_empty());
    assert_eq!(fake.calls(), 0);
    assert!(events(&e).iter().any(|l| l.contains(" failed  ") && l.ends_with("cancelled by user")));
    r.shutdown(true);
}

#[test]
fn redelegate_creates_the_linked_successor() {
    let fake = Fake::new(1);
    let e = ready(&fake);
    let old = new_record(&e, "retry me");
    edit(&old, |r| {
        r.state = State::Failed;
        r.requests.redelegate = true;
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
    assert!(new.context.contains("some context"));
    assert_ne!(new.id, old_rec.id);
    assert_eq!(new.state, State::Task);
    assert!(!new.requests.delegate);
    r.shutdown(true);
}

#[test]
fn requests_that_do_not_apply_are_ignored_and_cleared() {
    let fake = Fake::new(0);
    let e = ready(&fake);
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
fn a_key_planted_in_an_error_comes_out_redacted() {
    let fake = Fake::new(0);
    fake.status.store(500, Ordering::SeqCst);
    let e = ready(&fake);
    let path = new_record(&e, "will fail");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, lines) = runner(&e);
    r.start().unwrap();
    r.tick();
    r.shutdown(true);
    let all = lines.lock().unwrap().join("\n");
    assert!(all.contains("stopped at"), "{all}");
    assert!(!all.contains(KEY) && !all.contains("FAKEFAKE"), "{all}");
    assert!(all.contains("sk-ant-…x9Qa"));
    assert_eq!(fake.questions.load(Ordering::SeqCst), 0);
}

fn runner_with(e: &Env, f: impl FnOnce(Runner) -> Runner) -> (Runner, Arc<Mutex<Vec<String>>>) {
    let (r, lines) = runner(e);
    (f(r), lines)
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
    let fake = Fake::new(0);
    fake.status.store(500, Ordering::SeqCst);
    let e = ready(&fake);
    let path = new_record(&e, "will fail");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(rec.log.contains("define: error 1: "), "{}", rec.log);
    assert!(!rec.log.contains(KEY) && !rec.log.contains("FAKEFAKE"), "{}", rec.log);
    assert_eq!(fake.calls(), 3);
    let doc = registry_doc(&e);
    assert_eq!(doc["activity"].as_str(), Some("retry"));
    assert_eq!(doc["step"].as_str(), Some("define"));
    assert!(doc["retry_in_s"].as_u64().unwrap() > 0);
    let last = doc["last_error"].as_str().unwrap();
    assert!(last.contains("500") && !last.contains(KEY) && !last.contains("FAKEFAKE"), "{last}");
    r.tick();
    assert_eq!(fake.calls(), 3);
    r.shutdown(true);

    let (mut again, _) = runner(&e);
    again.start().unwrap();
    again.tick();
    assert_eq!(fake.calls(), 3, "the backoff must survive a restart");
    again.shutdown(true);

    let (mut eager, _) = runner_with(&e, |r| r.with_retry_base_s(0));
    eager.start().unwrap();
    eager.tick();
    assert!(load(&path).log.contains("define: error 2: "));
    eager.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Failed, None));
    assert!(rec.verdict.is_none() && !rec.waiting);
    assert!(rec.log.contains("define: failed after 3 errors"), "{}", rec.log);
    assert!(registry_files(&e).is_empty());
    let ev = events(&e);
    assert!(ev.iter().any(|l| l.contains(" failed  ") && l.contains("failed after 3 errors")), "{ev:?}");
    eager.tick();
    assert_eq!(fake.calls(), 9);
    eager.shutdown(true);
}

#[test]
fn the_model_retries_429_and_5xx_but_not_400() {
    let fake = Fake::new(0);
    fake.script.lock().unwrap().extend([429, 503]);
    let e = ready(&fake);
    let path = new_record(&e, "flaky model");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!(rec.state, State::Defined);
    assert_eq!(fake.calls(), 4);
    assert!(!rec.log.contains("error"));
    r.shutdown(true);

    let fake = Fake::new(0);
    fake.script.lock().unwrap().extend([400]);
    let e = ready(&fake);
    let path = new_record(&e, "bad request");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    assert_eq!(fake.calls(), 1);
    assert!(load(&path).log.contains("define: error 1: "));
    r.shutdown(true);
}

#[test]
fn a_response_cut_off_at_max_tokens_is_a_stage_error() {
    let fake = Fake::new(0);
    fake.cut.store(true, Ordering::SeqCst);
    let e = ready(&fake);
    let path = new_record(&e, "long answer");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(rec.log.contains("define: error 1: ") && rec.log.contains("max_tokens"), "{}", rec.log);
    assert!(fake.bodies.lock().unwrap()[0].contains("\"max_tokens\":4096"));
    r.shutdown(true);
}

#[test]
fn the_registry_entry_follows_the_task_from_gather_to_notify() {
    let fake = Fake::new(1);
    let e = ready(&fake);
    let path = new_record(&e, "write a greeting");
    edit(&path, |r| r.requests.delegate = true);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    r.tick();
    let doc = registry_doc(&e);
    assert_eq!(doc["state"].as_str(), Some("task"));
    assert_eq!(doc["step"].as_str(), Some("define"));
    assert_eq!(doc["waiting"].as_str(), Some("question"));
    assert_eq!(doc["activity"].as_str(), Some("parked"));
    assert!(doc["step_at"].as_str().unwrap().ends_with('Z'));
    assert!(doc["started_at"].as_str().unwrap().ends_with('Z'));
    assert!(doc["retry_in_s"].is_null() && doc["last_error"].is_null());
    assert_eq!(doc["usage"]["define"]["calls"].as_u64(), Some(1));
    assert_eq!(doc["usage"]["define"]["input_tokens"].as_u64(), Some(120));
    assert_eq!(doc["usage"]["define"]["output_tokens"].as_u64(), Some(45));
    assert!(doc["usage"]["define"]["usd"].as_f64().unwrap() > 0.0);
    assert_eq!(doc["question"]["text"].as_str(), Some("Which filename?"));
    let started = doc["started_at"].as_str().unwrap().to_string();
    let step_at = doc["step_at"].as_str().unwrap().to_string();

    edit(&path, |rec| {
        rec.question.as_mut().unwrap().answer =
            Some(Answer { value: Some("greet.md".into()), replacement: None, granted: None, by: Some("gui".into()), at: None });
    });
    r.tick();
    let doc = registry_doc(&e);
    assert_eq!(doc["state"].as_str(), Some("defined"));
    assert_eq!(doc["step"].as_str(), Some("approve"));
    assert_eq!(doc["waiting"].as_str(), Some("approval"));
    assert_eq!(doc["started_at"].as_str(), Some(started.as_str()));
    assert!(doc["step_at"].as_str().unwrap() >= step_at.as_str());
    assert_eq!(doc["usage"]["plan"]["calls"].as_u64(), Some(1));
    assert_eq!(doc["usage"]["define"]["calls"].as_u64(), Some(2));
    let cost = doc["cost_usd"].as_f64().unwrap();
    assert!((cost - load(&path).cost_usd).abs() < 1e-9, "{cost}");

    edit(&path, |rec| {
        let a = rec.approval.as_mut().unwrap();
        a.status = "approved".into();
        a.by = Some("gui".into());
    });
    r.tick();
    let rec = load(&path);
    assert!(rec.state == State::Failed && rec.step.is_none(), "{:?} {:?}", rec.state, rec.step);
    assert!(registry_files(&e).is_empty());
    r.shutdown(true);
}

#[test]
fn the_heartbeat_interval_comes_from_settings() {
    let fake = Fake::new(1);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
}

fn to_awaiting_approval(e: &Env, r: &mut Runner, title: &str) -> PathBuf {
    let path = new_record(e, title);
    edit(&path, |rec| rec.requests.delegate = true);
    r.tick();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
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
    let fake = Fake::new(0);
    let e = ready(&fake);
    let (mut r, _) = runner(&e);
    r.start().unwrap();
    let path = to_awaiting_approval(&e, &mut r, "pause me");
    let calls = fake.calls();
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
    assert_eq!((load(&path).state, fake.calls()), (State::Defined, calls));
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    assert_eq!(fake.calls(), 0);
    assert!(events(&e).iter().any(|l| l.contains(" done ")), "{:?}", events(&e));
    r.shutdown(true);
}

#[test]
fn complete_while_paused_succeeds() {
    let fake = Fake::new(0);
    let e = ready(&fake);
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
    let fake = Fake::new(0);
    let e = ready(&fake);
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
