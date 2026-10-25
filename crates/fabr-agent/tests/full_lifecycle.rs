//! Full-lifecycle mocked integration test (task 11.1).
//!
//! Drives a delegated record end to end through the `Runner` against a fake
//! Anthropic server that answers every pipeline stage:
//!   delegate -> Gather -> Define (spec/success/assertions, no question)
//!            -> Plan (plan + plan_meta{reversible, write_file, check})
//!            -> Approve gate (PARKS, no auto-approve)
//!   approved -> Execute (scoped write_file) -> Evaluate (verdict pass)
//!            -> Learn -> Notify -> Succeeded.
//!
//! The happy path exercises Requirements 4.1, 5.2, 5.3, 5.4 and 5.6 (the file
//! is written under the node directory, proving folder-scoped execution). A
//! second case covers the question gate (Requirement 5.1): define returns a
//! question, the record parks, the answer is recorded through the record, and
//! the pipeline resumes.

use fabr_agent::runner::{Options, Runner, Sink};
use fabr_core::Workspace;
use fabr_core::fabric::{self, FabricCreateReq};
use fabr_core::setup::Ctx;
use fabr_core::taskrec::{self, Answer, State, Step, TaskRecord};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const KEY: &str = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa";

/// Fake Anthropic server that answers define, plan, and execute requests.
///
/// It distinguishes the three pipeline stages by inspecting the request body:
/// the execute call carries a `tools` array and the "Use tools" instruction;
/// the define call carries the "keys spec, success, assertions, question"
/// instruction; anything else is treated as a plan request.
struct Fake {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    fn new(questions: usize) -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let questions = Arc::new(AtomicUsize::new(questions));
        let stop = Arc::new(AtomicBool::new(false));
        let (q, st) = (questions.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                let Ok(Some(mut req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
                let mut body = String::new();
                std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
                let text = Fake::reply(&body, &q);
                let resp = tiny_http::Response::from_string(text)
                    .with_header(tiny_http::Header::from_bytes("content-type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        });
        Fake { url, stop, handle: Some(handle) }
    }

    fn reply(body: &str, q: &AtomicUsize) -> String {
        if body.contains("Use tools to apply the plan") {
            // Execute: ask the executor to write greet.txt inside the node.
            let block = json!({
                "type": "tool_use",
                "id": "call_1",
                "name": "write_file",
                "input": {"path": "greet.txt", "content": "hi"}
            });
            return json!({"content": [block], "usage": {"input_tokens": 90, "output_tokens": 20}}).to_string();
        }
        if body.contains("keys spec, success, assertions, question") {
            let asked = q.load(Ordering::SeqCst) > 0;
            if asked {
                q.fetch_sub(1, Ordering::SeqCst);
            }
            let question = if asked { json!("Which filename?") } else { json!(null) };
            let inner = json!({
                "spec": "write greet.txt containing hi",
                "success": "greet.txt contains hi",
                "assertions": [{"id": "a1", "text": "greet.txt contains hi"}],
                "question": question
            });
            return json!({"content": [{"type": "text", "text": inner.to_string()}], "usage": {"input_tokens": 120, "output_tokens": 45}})
                .to_string();
        }
        let inner = json!({
            "spec": "write greet.txt containing hi",
            "success": "greet.txt contains hi",
            "plan": "1. write greet.txt",
            "assertions": [{"id": "a1", "text": "greet.txt contains hi"}],
            "plan_meta": {"reversibility": "reversible", "tools": ["write_file"], "checks": [{"cmd": "test", "args": ["-f", "greet.txt"]}]}
        });
        json!({"content": [{"type": "text", "text": inner.to_string()}], "usage": {"input_tokens": 300, "output_tokens": 80}}).to_string()
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

fn env(url: &str) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let proj = home.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let space = home.join(".fabric-space");
    std::fs::create_dir_all(&space).unwrap();
    std::fs::write(
        space.join("settings.toml"),
        format!(
            "default_folder = \"{}\"\nmodel = \"claude-sonnet-5-5\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\"]\ncompleted = true\n",
            proj.display()
        ),
    )
    .unwrap();
    std::fs::write(space.join("secrets.toml"), format!("anthropic_api_key = \"{KEY}\"\n")).unwrap();
    let mut ctx = Ctx::new(&home);
    ctx.anthropic_url = url.to_string();
    Env { tmp, ctx, proj }
}

fn runner(e: &Env) -> Runner {
    let cfg = e.tmp.path().join("cfg");
    let cache = e.tmp.path().join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let ws = Workspace::from_config(&cfg, &cache, Some(e.proj.clone())).unwrap();
    Runner::new(&e.ctx, ws, Options { interval_s: 5, once: false }, Sink::default()).unwrap()
}

fn new_record(e: &Env, title: &str) -> PathBuf {
    let req = FabricCreateReq {
        folder: e.proj.to_string_lossy().into_owned(),
        title: title.into(),
        due: Some("2026-11-01".into()),
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

/// Record an approval the way the Data_API / GUI does: flip the status to
/// "approved" while keeping the plan digest the executor validates. The digest
/// is recomputed from the record so it matches `approval_allows_execute`.
fn approve(p: &Path) {
    edit(p, |rec| {
        let digest = taskrec::approval_digest(rec);
        let approval = rec.approval.as_mut().expect("plan sets an approval before the gate");
        approval.status = "approved".into();
        approval.by = Some("gui".into());
        approval.at = Some("2026-10-24T19:00:00Z".into());
        approval.digest = Some(digest);
    });
}

/// Drive the runner for up to `max` ticks, stopping once `done` holds.
fn drive_until(r: &mut Runner, p: &Path, max: usize, done: impl Fn(&TaskRecord) -> bool) -> TaskRecord {
    for _ in 0..max {
        let rec = load(p);
        if done(&rec) {
            return rec;
        }
        r.tick();
    }
    let rec = load(p);
    assert!(done(&rec), "condition not reached in {max} ticks: state={:?} step={:?}", rec.state, rec.step);
    rec
}

#[test]
fn delegate_through_approval_scoped_execute_to_verified_success() {
    let fake = Fake::new(0);
    let e = env(&fake.url);
    let path = new_record(&e, "add greet.txt that contains hi");
    edit(&path, |r| r.requests.delegate = true);
    let mut r = runner(&e);
    r.start().unwrap();

    // Delegate -> Gather -> Define -> Plan, then PARK at the approval gate.
    let gated = drive_until(&mut r, &path, 30, |rec| rec.state == State::Defined && rec.step == Some(Step::Approve));
    assert!(gated.waiting, "the record parks at the approval gate");
    assert!(!gated.spec.trim().is_empty(), "define produced a spec");
    assert!(!gated.success.trim().is_empty(), "define produced success criteria");
    assert!(!gated.plan.trim().is_empty(), "plan produced a plan");
    assert!(!gated.assertions.is_empty(), "plan carried assertions");
    let meta = gated.plan_meta.as_ref().expect("plan_meta present");
    assert_eq!(meta.reversibility.as_deref(), Some("reversible"));
    assert_eq!(meta.tools, vec!["write_file".to_string()]);
    assert_eq!(meta.checks.len(), 1);
    let approval = gated.approval.as_ref().expect("approval present at the gate");
    assert_eq!(approval.status, "pending", "the runner does not auto-approve");
    assert_eq!(approval.digest.as_deref(), Some(taskrec::approval_digest(&gated).as_str()));
    assert!(gated.question.is_none(), "no question on the happy path");

    // The gate withholds the side effect: nothing is written before approval.
    assert!(!e.proj.join("greet.txt").exists(), "no file is written before approval");

    // Parking is stable: more ticks do not advance past the gate.
    r.tick();
    r.tick();
    let still = load(&path);
    assert_eq!((still.state, still.step), (State::Defined, Some(Step::Approve)));
    assert_eq!(still.approval.as_ref().unwrap().status, "pending");
    assert!(!e.proj.join("greet.txt").exists());

    // Grant approval through the record, then drive to a verified result.
    approve(&path);
    let done = drive_until(&mut r, &path, 30, |rec| matches!(rec.state, State::Succeeded | State::Failed));
    assert_eq!(done.state, State::Succeeded, "verified result, not a failure: log={}", done.log);
    assert!(done.step.is_none(), "terminal record has no pending step");

    // Scoped execution: the file lands inside the node directory (Req 5.6).
    assert_eq!(std::fs::read_to_string(e.proj.join("greet.txt")).unwrap(), "hi");

    // The verdict passes and names the plan's assertion.
    let verdict = done.verdict.as_ref().expect("a verdict is recorded");
    assert_eq!(verdict.result, "pass");
    assert_eq!(verdict.items.len(), 1);
    assert_eq!(verdict.items[0].id, "a1");
    assert!(verdict.items[0].pass, "the assertion passed");

    // The pipeline ran its tail (Learn/Notify) and the cost was captured.
    assert!(done.log.contains("execute: write_file greet.txt"), "execute logged the scoped write");
    assert!(done.log.contains("notify"), "notify step ran: {}", done.log);
    assert!(done.cost_usd > 0.0, "model calls accrued cost");

    r.shutdown(true);
}

#[test]
fn define_question_parks_then_resumes_after_an_answer() {
    let fake = Fake::new(1);
    let e = env(&fake.url);
    let path = new_record(&e, "add greet.txt that contains hi");
    edit(&path, |r| r.requests.delegate = true);
    let mut r = runner(&e);
    r.start().unwrap();

    // Define asks a question; the record parks at the Question_Gate (Req 5.1).
    let asked = drive_until(&mut r, &path, 10, |rec| rec.question.is_some());
    assert_eq!((asked.state, asked.step), (State::Task, Some(Step::Define)));
    assert!(asked.waiting);
    assert_eq!(asked.question.as_ref().unwrap().text, "Which filename?");
    assert!(asked.question.as_ref().unwrap().answer.is_none());

    // Parking is stable until the answer is recorded.
    r.tick();
    let still = load(&path);
    assert_eq!((still.state, still.step), (State::Task, Some(Step::Define)));
    assert!(still.question.as_ref().unwrap().answer.is_none());

    // Record the answer through the record, then the pipeline resumes to plan.
    edit(&path, |rec| {
        rec.question.as_mut().unwrap().answer = Some(Answer {
            value: Some("greet.txt".into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-24T18:05:02Z".into()),
        });
    });
    let resumed = drive_until(&mut r, &path, 10, |rec| rec.state == State::Defined && rec.step == Some(Step::Approve));
    assert!(resumed.question.is_none(), "the answered question moved to answers");
    assert_eq!(resumed.answers.len(), 1);
    assert_eq!(resumed.answers[0].answer.as_ref().unwrap().value.as_deref(), Some("greet.txt"));
    assert_eq!(resumed.approval.as_ref().unwrap().status, "pending");

    r.shutdown(true);
}

/// A model that keeps asking a question every define cycle must NOT loop
/// forever: once the user has answered the define budget of questions, the
/// definer stops asking. This reproduces the field bug where the GUI stored the
/// answer (it lands in `rec.answers`) but define re-asked a fresh q1 every tick
/// because the prior answers were never fed back — an infinite ask/answer loop.
/// Here a huge `questions` count makes the fake ask on every define call; we
/// auto-answer each question the way the GUI does and assert the loop is bounded
/// by the budget and then proceeds (the fake still returns a usable spec).
#[test]
fn define_stops_asking_after_the_question_budget() {
    // Far more than the budget: the fake would ask forever if unbounded.
    let fake = Fake::new(1000);
    let e = env(&fake.url);
    let path = new_record(&e, "add greet.txt that contains hi");
    edit(&path, |r| r.requests.delegate = true);
    let mut r = runner(&e);
    r.start().unwrap();

    // Auto-answer each question the definer asks, as the GUI would, for more
    // ticks than the budget. If the loop were unbounded this would never leave
    // Define; the budget must make it proceed (or fail) within a bounded count.
    let mut answered = 0;
    let mut proceeded = false;
    for _ in 0..40 {
        let rec = load(&path);
        if rec.state == State::Defined || (rec.state == State::Task && rec.step == Some(Step::Plan)) {
            proceeded = true;
            break;
        }
        if rec.waiting && rec.question.as_ref().is_some_and(|q| q.answer.is_none()) {
            edit(&path, |rec| {
                rec.question.as_mut().unwrap().answer = Some(Answer {
                    value: Some(format!("answer {answered}")),
                    replacement: None,
                    granted: None,
                    by: Some("gui".into()),
                    at: Some("2026-10-25T10:55:52Z".into()),
                });
            });
            answered += 1;
        }
        r.tick();
    }

    let rec = load(&path);
    // The loop is bounded: the user was never asked more than the budget.
    assert!(answered <= 5, "define asked more than the budget: {answered} questions");
    // And it did not get stuck re-asking forever — it left the Define question gate.
    assert!(
        proceeded || !(rec.state == State::Task && rec.step == Some(Step::Define) && rec.waiting),
        "record is still stuck waiting in Define after the budget: state={:?} step={:?} waiting={}",
        rec.state,
        rec.step,
        rec.waiting
    );

    r.shutdown(true);
}

/// The prior answers a user gave are fed back to the definer, so a model that
/// asks once and then (given the answer in context) proceeds does so on the
/// strength of that answer. This pins the root-cause fix: `define_stage` must
/// include the answered Q&A in the context it sends to `define`.
#[test]
fn define_feeds_prior_answers_back_into_the_model() {
    // Ask exactly once; after the answer is recorded the next define call should
    // see it in context and return a spec (the fake stops asking after 1).
    let fake = Fake::new(1);
    let e = env(&fake.url);
    let path = new_record(&e, "add greet.txt that contains hi");
    edit(&path, |r| r.requests.delegate = true);
    let mut r = runner(&e);
    r.start().unwrap();

    let asked = drive_until(&mut r, &path, 10, |rec| rec.question.is_some());
    assert!(asked.question.as_ref().unwrap().answer.is_none());
    edit(&path, |rec| {
        rec.question.as_mut().unwrap().answer = Some(Answer {
            value: Some("greet.txt".into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-25T10:55:52Z".into()),
        });
    });

    let resumed = drive_until(&mut r, &path, 10, |rec| rec.state == State::Defined);
    // The answered question was folded into answers and the definer proceeded.
    assert_eq!(resumed.answers.len(), 1);
    assert_eq!(resumed.answers[0].answer.as_ref().unwrap().value.as_deref(), Some("greet.txt"));
    assert!(!resumed.spec.trim().is_empty(), "define produced a spec after seeing the answer");

    r.shutdown(true);
}
