mod common;

use common::*;
use fabr_agent::claude::{Call, ClaudeEngine, Role, abort_all, allow_runs, live_runs, session};
use fabr_agent::{End, Watch};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

struct Quiet;
impl Watch for Quiet {}

fn call(cwd: &std::path::Path) -> Call<'_> {
    Call {
        role: Role::Validator,
        cwd,
        resume: None,
        prompt: "x".into(),
        schema: json!({"type": "object"}),
        allowed: vec![],
        name: String::new(),
    }
}

#[test]
fn abort_all_stops_a_live_run_and_blocks_new_ones_until_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = FakeClaude::new(tmp.path());
    fake.push(vec![init(), "@SLEEP 5".into(), "@SH touch late.md".into()]);
    fake.push(ok(json!({"ok": true}), 0.01));
    let engine = ClaudeEngine::new(&fake.bin).with_timeout(Duration::from_secs(30));
    let cwd = tmp.path().to_path_buf();
    let started = Instant::now();
    let worker = {
        let (engine, cwd) = (engine.clone(), cwd.clone());
        std::thread::spawn(move || matches!(session::<Value>(&engine, &call(&cwd), &mut Quiet).end, End::Stopped))
    };
    while live_runs() == 0 && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    abort_all();
    assert!(worker.join().unwrap(), "an aborted run ends as stopped");
    assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
    assert_eq!(live_runs(), 0);
    assert!(matches!(session::<Value>(&engine, &call(&cwd), &mut Quiet).end, End::Stopped));
    assert_eq!(fake.calls(), 1, "nothing is spawned while aborted");
    allow_runs();
    assert!(matches!(session::<Value>(&engine, &call(&cwd), &mut Quiet).end, End::Done(_)));
    std::thread::sleep(Duration::from_millis(5500).saturating_sub(started.elapsed()));
    assert!(!cwd.join("late.md").exists(), "the killed group kept running");
}
