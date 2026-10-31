use std::path::Path;
use std::process::{Command, Output};

fn fabr(home: &Path, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fabr"));
    c.args(args);
    Command::env(&mut c, "FABR_HOME", home);
    c.output().unwrap()
}

#[test]
fn status_json_and_answer_use_the_default_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let proj = home.join("proj");
    let tasks = proj.join(".fabric-space/tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(home.join(".fabric-space")).unwrap();
    std::fs::write(home.join(".fabric-space/settings.toml"), format!("default_folder = \"{}\"\n", proj.display())).unwrap();
    let rec = "---\nschema: 7\nid: 2026-10-24-pick-colour\nsource: null\ntitle: Pick colour\ndue: null\npri: 0\nstate: task\nstep: define\nwaiting: true\npaused: false\nsuspended: false\nsuspend_until: null\ncreated_at: 2026-10-24T10:00:00Z\nupdated_at: 2026-10-24T10:00:00Z\nrequests: {delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null}\nquestion: {id: q1, kind: open, text: Which colour?, options: null, prior: null, fields: null, asked_at: null, answer: null}\nanswers: []\ngrants: {requested: [], granted: []}\nplan_meta: null\nassertions: []\napproval: null\nverdict: null\ncost_usd: 0\nlinks: {redelegated_from: null, redelegated_to: null}\nby: {context: null, spec: null, success: null, plan: null}\nneeds: null\nsessions: {plan: null, execute: null, evaluate: null}\n---\n## Context\n## Spec\n## Success\n## Plan\n## Log\n";
    let file = tasks.join("2026-10-24-pick-colour.md");
    std::fs::write(&file, rec).unwrap();

    let out = fabr(&home, &["status", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (v[0]["waiting"].as_str(), v[0]["question"].as_str(), v[0]["state"].as_str()),
        (Some("question"), Some("Which colour?"), Some("task"))
    );

    let out = fabr(&home, &["answer", "pick-colour", "dark", "red"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "answer recorded\n");
    assert!(std::fs::read_to_string(&file).unwrap().contains("dark red"));

    let out = fabr(&home, &["answer", file.to_str().unwrap(), "again"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already answered"));
}

#[test]
fn track_adds_a_claude_code_session_as_a_reminder() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let proj = tmp.path().join("proj");
    let cfg = tmp.path().join("claude");
    std::fs::create_dir_all(&proj).unwrap();
    let id = "2e6225e8-f0da-4b18-8eca-40b44d4bae82";
    let dir = cfg.join("projects").join(fabr_core::ccsession::encode_folder(&proj.canonicalize().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.jsonl")), r#"{"type":"user","message":{"content":"Sort the scans"}}"#).unwrap();
    let run = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_fabr"));
        c.args(["track", proj.to_str().unwrap(), id]);
        Command::env(&mut c, "FABR_HOME", &home);
        Command::env(&mut c, "CLAUDE_CONFIG_DIR", &cfg);
        c.output().unwrap()
    };
    let out = run();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let path = String::from_utf8(out.stdout).unwrap();
    let text = std::fs::read_to_string(path.trim()).unwrap();
    assert!(text.contains("title: Sort the scans") && text.contains(&format!("Picked up from Claude Code session {id}.")));
    assert!(!run().status.success());
}
