mod common;

use common::{FakeClaude, init, tool_use};
use fabr_agent::engine::NoWatch;
use fabr_agent::lifecycle::advance_with;
use fabr_agent::link::open_reminder;
use fabr_agent::{ClaudeEngine, Ctl, DefineInput, End, Engine};
use fabr_core::taskrec::{self, State, Step, TaskRecord};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn load(p: &Path) -> TaskRecord {
    taskrec::parse(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn edit(p: &Path, f: impl Fn(&mut TaskRecord)) {
    for _ in 0..50 {
        let mut rec = load(p);
        let expected = rec.updated_at.clone();
        f(&mut rec);
        if taskrec::write_record(p, &rec, Some(&expected)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("could not edit {}", p.display());
}

struct Node {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    space: PathBuf,
    fake: FakeClaude,
}

fn node() -> Node {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("proj");
    let space = tmp.path().join("space");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&space).unwrap();
    let fake = FakeClaude::new(tmp.path());
    Node { dir, space, fake, _tmp: tmp }
}

fn running(n: &Node) -> PathBuf {
    let path = open_reminder(&n.dir, "long job").unwrap();
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.plan = "### Inputs\n-\n### Outputs\n-\n### Steps\n1. work".into();
    });
    path
}

#[test]
fn a_cancel_kills_the_running_claude_process_group_promptly() {
    let n = node();
    n.fake.push(vec![init(), tool_use("Bash", json!({"command": "sleep 4"})), "@SLEEP 4".into(), "@SH touch late.md".into()]);
    let path = running(&n);
    let engine = ClaudeEngine::new(&n.fake.bin);
    let ctl = Ctl::new(&n.space, 10);
    let flag = path.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        edit(&flag, |r| r.requests.cancel = true);
    });
    let started = Instant::now();
    advance_with(&n.dir, &path, &ctl, &engine).unwrap();
    let took = started.elapsed();
    canceller.join().unwrap();
    assert!(took < Duration::from_millis(3800), "{took:?}");
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Failed, None));
    assert!(rec.log.contains("execute: Bash sleep 4") && rec.log.contains("execute: run stopped"), "{}", rec.log);
    assert!(rec.log.contains("execute: cancelled by user"), "{}", rec.log);
    assert!(rec.sessions.execute.is_some());
    std::thread::sleep(Duration::from_millis(4500).saturating_sub(started.elapsed()));
    assert!(!n.dir.join("late.md").exists(), "the fake kept running after the kill");
}

#[test]
fn a_pause_stops_the_run_and_resume_continues_the_same_session() {
    let n = node();
    n.fake.push(vec![init(), "@SLEEP 5".into()]);
    n.fake.push(common::ok(common::exec_out("done", "finished"), 0.01));
    let path = running(&n);
    let engine = ClaudeEngine::new(&n.fake.bin);
    let ctl = Ctl::new(&n.space, 10);
    let flag = path.clone();
    let pauser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        edit(&flag, |r| r.requests.pause = true);
    });
    advance_with(&n.dir, &path, &ctl, &engine).unwrap();
    pauser.join().unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::InProgress, Some(Step::Execute)));
    assert!(rec.log.contains("execute: run stopped"), "{}", rec.log);
    let sid = rec.sessions.execute.clone().unwrap();
    assert_eq!(n.fake.flag(1, "--session-id").as_deref(), Some(sid.as_str()));
    edit(&path, |r| r.requests.pause = false);
    advance_with(&n.dir, &path, &ctl, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(rec.step, Some(Step::Evaluate), "{}", rec.log);
    assert_eq!(n.fake.flag(2, "--resume").as_deref(), Some(sid.as_str()));
    assert!(n.fake.stdin(2).contains("Continue the task"));
}

#[test]
fn a_run_past_the_timeout_is_killed_and_is_a_stage_error() {
    let n = node();
    n.fake.push(vec![init(), "@SLEEP 5".into()]);
    let engine = ClaudeEngine::new(&n.fake.bin).with_timeout(Duration::from_secs(1));
    let started = Instant::now();
    let run = engine.define(&DefineInput { cwd: &n.dir, resume: None, title: "t", context: "", answers: "" }, &mut NoWatch);
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    let End::Failed(err) = run.end else { panic!("{:?}", run.end) };
    assert!(err.message().contains("ran longer than 1s"), "{}", err.message());
    assert!(run.session_id.is_some());
}

#[test]
fn a_missing_binary_is_a_stage_error_not_a_crash() {
    let n = node();
    let path = running(&n);
    let engine = ClaudeEngine::new(n.dir.join("no-such-claude"));
    let err = advance_with(&n.dir, &path, &Ctl::new(&n.space, 10), &engine).unwrap_err();
    assert!(err.message().contains("could not start Claude Code"), "{}", err.message());
    assert!(load(&path).log.contains("execute: error 1: could not start Claude Code"));
}

#[test]
fn the_read_only_executor_gets_no_write_tools() {
    let n = node();
    n.fake.push(common::ok(common::exec_out("done", "looked"), 0.0));
    let path = running(&n);
    edit(&path, |r| {
        r.plan_meta = Some(taskrec::PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec!["Read".into()],
            assumptions: vec![],
            checks: vec![],
            requires: Default::default(),
        })
    });
    let mut ctl = Ctl::new(&n.space, 10);
    ctl.harness.mode = fabr_core::setup::ExecuteMode::ReadOnly;
    advance_with(&n.dir, &path, &ctl, &ClaudeEngine::new(&n.fake.bin)).unwrap();
    assert_eq!(n.fake.flag(1, "--tools").as_deref(), Some("Read,Glob,Grep"));
    assert_eq!(n.fake.flag(1, "--allowedTools").as_deref(), Some("Read Glob Grep"));
    assert!(n.fake.stdin(1).contains("read-only"));
}

#[test]
fn live_claude_code_runs_a_tiny_task_end_to_end() {
    if std::env::var("FABR_LIVE_CLAUDE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set FABR_LIVE_CLAUDE=1 to run against the real claude binary");
        return;
    }
    let bin = fabr_agent::resolve_bin("").expect("claude on PATH");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("live");
    let space = tmp.path().join("space");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&space).unwrap();
    let path = open_reminder(&dir, "Create hello.md in this folder containing exactly the word hi").unwrap();
    edit(&path, |r| r.requests.delegate = true);
    let engine = ClaudeEngine::new(bin).with_model("haiku", "").with_timeout(Duration::from_secs(300));
    let mut ctl = Ctl::new(&space, 10);
    ctl.harness.auto_mode = true;
    ctl.root = Some(dir.clone());
    for _ in 0..24 {
        let rec = load(&path);
        if rec.step.is_none() && rec.state != State::Reminder {
            break;
        }
        if rec.question.as_ref().is_some_and(|q| q.answer.is_none()) {
            eprintln!("question: {}", rec.question.as_ref().unwrap().text);
            edit(&path, |r| {
                r.question.as_mut().unwrap().answer = Some(taskrec::Answer {
                    value: Some("allow. Use your best judgment: create hello.md with the single word hi.".into()),
                    replacement: None,
                    granted: None,
                    by: Some("gui".into()),
                    at: None,
                })
            });
        }
        if let Err(e) = advance_with(&dir, &path, &ctl, &engine) {
            eprintln!("stage error: {}", e.message());
        }
    }
    let rec = load(&path);
    eprintln!("{}\nplan:\n{}\ncost_usd={:.4} sessions={:?}", rec.log, rec.plan, rec.cost_usd, rec.sessions);
    assert_eq!(rec.state, State::Succeeded, "{}", rec.log);
    assert_eq!(std::fs::read_to_string(dir.join("hello.md")).unwrap().trim(), "hi");
    assert!(rec.sessions.plan.is_some() && rec.sessions.execute.is_some() && rec.sessions.evaluate.is_some());
    assert!(rec.cost_usd > 0.0);
}
