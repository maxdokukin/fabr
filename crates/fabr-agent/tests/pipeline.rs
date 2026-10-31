mod common;

use fabr_agent::engine::NoWatch;
use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::{
    DefineInput, DefineOut, DraftAssertion, DraftCheck, End, Engine, EvaluateInput, EvaluateOut, ExecuteInput, ExecuteOut, JudgedItem,
    Judgement, PlanInput, PlanOut, ProposeInput, ProposeOut, Run, ScriptedEngine, ScriptedExec, Seen, ToolUse, Usage, Watch,
    approval_allows_execute, run_once,
};
use fabr_core::fabric::{self, FabricWriteReq};
use fabr_core::taskrec::{self, State, Step, TaskRecord};
use fabr_core::{NodeRef, Phase, Workspace};
use serde_json::json;
use std::path::{Path, PathBuf};

fn workspace(root: &Path) -> Workspace {
    let cfg = root.join("cfg");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    Workspace::from_config(&cfg, &cache, Some(root.to_path_buf())).unwrap()
}

fn setup() -> (tempfile::TempDir, PathBuf, Workspace) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let ws = workspace(&root);
    ws.init_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let fabric = tmp.path().join("fabric-space");
    std::fs::create_dir_all(&fabric).unwrap();
    std::fs::write(
        fabric.join("settings.toml"),
        "default_folder = \"/\"\nmodel = \"haiku\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"model\"]\n",
    )
    .unwrap();
    (tmp, fabric, ws)
}

fn root_of(ws: &Workspace) -> PathBuf {
    ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap()
}

fn defined() -> DefineOut {
    DefineOut {
        question: None,
        spec: "Write greet.md containing hi".into(),
        success: "greet.md contains hi".into(),
        assertions: vec![DraftAssertion { id: "a1".into(), text: "greet.md contains hi".into() }],
        ..Default::default()
    }
}

fn asks(q: &str) -> DefineOut {
    DefineOut { question: Some(q.into()), ..Default::default() }
}

const PLAN: &str = "### Inputs\nnone\n\n### Outputs\ngreet.md\n\n### Steps\n1. Write greet.md";

fn plan(text: &str, tools: &[&str], check: (&str, &[&str])) -> PlanOut {
    PlanOut {
        plan: text.into(),
        reversibility: "reversible".into(),
        tools: tools.iter().map(|t| (*t).to_string()).collect(),
        checks: vec![DraftCheck { cmd: check.0.into(), args: check.1.iter().map(|a| (*a).to_string()).collect() }],
    }
}

fn greet_plan() -> PlanOut {
    plan(PLAN, &["Write"], ("test", &["-f", "greet.md"]))
}

fn writes_greet() -> ScriptedExec {
    ScriptedExec::done("wrote greet.md").write("greet.md", "hi")
}

fn greet() -> ScriptedEngine {
    ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(writes_greet())
}

fn record_path(root: &Path) -> PathBuf {
    let tasks = root.join(".fabric-space").join("tasks");
    std::fs::read_dir(&tasks).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "md")).unwrap()
}

fn load(path: &Path) -> taskrec::TaskRecord {
    taskrec::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn edit(path: &Path, f: impl FnOnce(&mut TaskRecord)) {
    let mut rec = load(path);
    let expected = rec.updated_at.clone();
    f(&mut rec);
    taskrec::write_record(path, &rec, Some(&expected)).unwrap();
}

fn save_one(ws: &Workspace, title: &str) -> PathBuf {
    let path = open_reminder(&root_of(ws), title).unwrap();
    edit(&path, |r| r.requests.delegate = true);
    path
}

fn approve(path: &Path) {
    let mut rec = load(path);
    let approval = rec.approval.as_mut().unwrap();
    approval.status = "approved".into();
    approval.by = Some("gui".into());
    approval.at = Some("2026-10-24T00:00:00Z".into());
    let expected = rec.updated_at.clone();
    taskrec::write_record(path, &rec, Some(&expected)).unwrap();
}

fn answer(path: &Path, value: &str) {
    edit(path, |r| {
        r.question.as_mut().unwrap().answer =
            Some(taskrec::Answer { value: Some(value.into()), replacement: None, granted: None, by: Some("gui".into()), at: None })
    });
}

#[test]
fn fixture_gather_persists_through_the_record_writer() {
    let (_tmp, _fabric, ws) = setup();
    let root = root_of(&ws);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/01-reminder/project/.fabric-space/tasks/2026-10-24-check-printer-firmware.md");
    let rec = taskrec::parse(&std::fs::read_to_string(&fixture).unwrap()).unwrap();
    let path = root.join(".fabric-space").join("tasks").join(format!("{}.md", rec.id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut rec = rec;
    rec.requests.delegate = true;
    taskrec::write_record(&path, &rec, None).unwrap();
    let engine = greet();
    advance(&root, &path, &root, 10, &engine).unwrap();
    advance(&root, &path, &root, 10, &engine).unwrap();
    let again = load(&path);
    assert_eq!((again.state, again.step), (State::Task, Some(Step::Define)));
    assert!(again.context.contains("Release notes are on the maker site."));
    assert!(again.context.contains("### Gathered") && again.context.contains("#### Files"));
    let round = taskrec::parse(&taskrec::render(&again)).unwrap();
    assert_eq!(round.context, again.context);
}

#[test]
fn gather_merges_the_folder_chain_rules_into_the_context() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let sub = root.join("errands");
    std::fs::create_dir_all(&sub).unwrap();
    ws.init_node(NodeRef { source: "local".into(), path: "errands".into() }).unwrap();
    for (dir, text) in [(&root, "## Always\nanswer in English"), (&sub, "keep receipts in receipts/")] {
        std::fs::create_dir_all(dir.join(".fabric-space/.agents")).unwrap();
        std::fs::write(dir.join(".fabric-space/.agents/rules.md"), text).unwrap();
    }
    std::fs::write(sub.join("list.md"), "milk").unwrap();
    let path = open_reminder(&sub, "shop").unwrap();
    edit(&path, |r| r.requests.delegate = true);
    let engine = ScriptedEngine::new().define(asks("Which shop?"));
    run_once(&ws, "local", "errands", &fabric, &engine).unwrap();
    let rec = load(&path);
    let near = rec.context.find("From errands (this folder):\nkeep receipts").unwrap();
    let far = rec.context.find("From .:\n##### Always\nanswer in English").unwrap();
    assert!(near < far, "{}", rec.context);
    assert!(rec.context.contains("list.md: milk"), "{}", rec.context);
    let Seen::Define { context, resume, .. } = &engine.seen()[0] else { panic!() };
    assert!(context.contains("keep receipts") && resume.is_none(), "{context}");
}

#[test]
fn registry_names_differ_for_the_same_slug() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let a = left.path().join(".fabric-space").join("tasks").join("2026-10-24-alpha.md");
    let b = right.path().join(".fabric-space").join("tasks").join("2026-10-24-alpha.md");
    assert_ne!(
        taskrec::registry_filename("2026-10-24-alpha", &a.to_string_lossy()),
        taskrec::registry_filename("2026-10-24-alpha", &b.to_string_lossy())
    );
}

#[test]
fn the_planner_session_defines_then_plans_and_stops_at_pending_approval() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet().with_cost(0.25);
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert!(rec.waiting && !rec.spec.trim().is_empty() && !rec.success.trim().is_empty());
    assert!(rec.plan.contains("### Inputs") && rec.plan.contains("### Outputs") && rec.plan.contains("### Steps"));
    let meta = rec.plan_meta.as_ref().unwrap();
    assert_eq!((meta.tools.clone(), meta.checks.len()), (vec!["Write".to_string()], 1));
    assert_eq!(meta.reversibility.as_deref(), Some("reversible"));
    assert_eq!(rec.approval.as_ref().unwrap().digest.clone().unwrap(), taskrec::approval_digest(&rec));
    let sid = rec.sessions.plan.clone().unwrap();
    assert!(rec.sessions.execute.is_none() && rec.sessions.evaluate.is_none());
    let seen = engine.seen();
    assert!(matches!(&seen[0], Seen::Define { resume: None, .. }));
    assert!(matches!(&seen[1], Seen::Plan { resume: Some(s), .. } if *s == sid), "{seen:?}");
    assert!((rec.cost_usd - 0.5).abs() < 1e-9, "{}", rec.cost_usd);
    assert_eq!(engine.count("execute"), 0);
    assert!(!root.join("greet.md").exists());
}

#[test]
fn define_question_pauses_without_a_new_state() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(asks("Which filename?"));
    save_one(&ws, "ask first");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
    assert!(rec.waiting);
    assert_eq!(rec.question.as_ref().unwrap().text, "Which filename?");
    assert!(rec.question.as_ref().unwrap().answer.is_none() && rec.spec.trim().is_empty() && rec.plan.trim().is_empty());
    assert!(rec.sessions.plan.is_some());
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(engine.count("define"), 1);
}

#[test]
fn the_users_answer_resumes_the_planner_session_for_define_and_plan() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(asks("Which filename?")).define(defined()).plan(greet_plan());
    let path = save_one(&ws, "job");
    for _ in 0..3 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
    }
    assert_eq!(load(&path).question.as_ref().unwrap().id, "q1");
    let sid = load(&path).sessions.plan.clone().unwrap();
    answer(&path, "call it hello-42.txt");
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let seen = engine.seen();
    assert_eq!(seen.len(), 3, "{seen:?}");
    let Seen::Define { resume, answers, .. } = &seen[1] else { panic!("{seen:?}") };
    assert_eq!(resume.as_deref(), Some(sid.as_str()));
    assert!(answers.contains("Already asked: Which filename?\nUser answered: call it hello-42.txt"), "{answers}");
    let Seen::Plan { resume, answers, .. } = &seen[2] else { panic!("{seen:?}") };
    assert_eq!(resume.as_deref(), Some(sid.as_str()));
    assert!(answers.contains("User answered: call it hello-42.txt"));
    let done = load(&path);
    assert_eq!((done.answers.len(), done.state, done.sessions.plan.as_deref()), (1, State::Defined, Some(sid.as_str())));
}

#[test]
fn a_plan_still_invalid_after_one_repair_round_parks_with_needs_and_no_error_count() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(defined()).plan(plan(PLAN, &["browser"], ("test", &["-f", "greet.md"]))).plan(plan(
        PLAN,
        &["browser"],
        ("git", &["status"]),
    ));
    save_one(&ws, "browse");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rejections: Vec<Option<String>> =
        engine.seen().into_iter().filter_map(|s| if let Seen::Plan { rejection, .. } = s { Some(rejection) } else { None }).collect();
    assert_eq!(rejections.len(), 2);
    assert!(rejections[0].is_none());
    let sent = rejections[1].as_deref().unwrap();
    assert!(sent.contains("unsupported tool browser") && sent.contains("allowed checks: test") && sent.contains("Bash"), "{sent}");
    let rec = load(&record_path(&root));
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!(rec.needs.as_deref(), Some("harness: browser, checks: git"));
    assert!(rec.log.contains("plan: needs harness: browser, checks: git") && !rec.log.contains("error"), "{}", rec.log);
    assert_eq!(engine.count("execute"), 0);
}

#[test]
fn a_repair_round_fixes_a_disallowed_check_or_a_bare_bash() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(defined()).plan(plan(PLAN, &["Bash"], ("git", &["status"]))).plan(greet_plan());
    save_one(&ws, "repair");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&record_path(&root));
    assert!(rec.needs.is_none() && rec.log.contains("plan: retry after rejection: unsupported tool Bash"), "{}", rec.log);
    assert_eq!(rec.plan_meta.unwrap().requires.checks, vec!["test".to_string()]);
}

#[test]
fn a_plan_without_inputs_outputs_and_steps_is_sent_back_then_is_a_stage_error() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let bad = || plan("1. write greet.md", &["Write"], ("test", &["-f", "greet.md"]));
    let engine = ScriptedEngine::new().define(defined()).plan(bad()).plan(bad());
    save_one(&ws, "shape");
    assert!(run_once(&ws, "local", "", &fabric, &engine).is_err());
    let rec = load(&record_path(&root));
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Plan)));
    assert!(rec.log.contains("plan: retry after rejection: the plan is missing ### Inputs, ### Outputs, ### Steps"), "{}", rec.log);
    assert!(rec.log.contains("plan: error 1: the plan is missing"), "{}", rec.log);
    let fixed = ScriptedEngine::new().plan(greet_plan());
    run_once(&ws, "local", "", &fabric, &fixed).unwrap();
    assert_eq!(load(&record_path(&root)).state, State::Defined);
}

#[test]
fn a_shell_or_interpreter_check_never_reaches_an_approvable_plan() {
    for (cmd, args) in [("sh", vec!["-c", "id"]), ("env", vec!["sh"]), ("python3", vec!["-c", "1"]), ("cat", vec!["/etc/passwd"])] {
        let (_tmp, fabric, ws) = setup();
        let root = root_of(&ws);
        let bad = || plan(PLAN, &["Write"], (cmd, &args));
        let engine = ScriptedEngine::new().define(defined()).plan(bad()).plan(bad()).exec(writes_greet());
        save_one(&ws, "check");
        run_once(&ws, "local", "", &fabric, &engine).unwrap();
        assert!(load(&record_path(&root)).needs.is_some(), "{cmd}");
        edit(&record_path(&root), |r| r.approval.as_mut().unwrap().status = "approved".into());
        let _ = run_once(&ws, "local", "", &fabric, &engine);
        assert_eq!(load(&record_path(&root)).state, State::Defined, "{cmd}");
        assert_eq!(engine.count("execute"), 0, "{cmd}");
    }
}

#[test]
fn approval_refuses_pending_refused_and_stale_digest() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    save_one(&ws, "gate");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(load(&path).state, State::Defined);
    assert_eq!(engine.count("execute"), 0);
    let sid = load(&path).sessions.plan.clone();

    edit(&path, |rec| {
        rec.approval.as_mut().unwrap().status = "refused".into();
        rec.approval.as_mut().unwrap().note = Some("shorter plan".into());
    });
    engine.push_plan(Ok(plan(&format!("{PLAN} differently"), &["Write"], ("test", &["-f", "greet.md"]))));
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let redrafted = load(&path);
    assert_eq!(redrafted.state, State::Defined);
    assert_eq!(redrafted.approval.as_ref().unwrap().status, "pending");
    assert!(redrafted.plan.contains("differently") && redrafted.log.contains("refused: shorter plan"));
    assert_eq!(redrafted.approval.as_ref().unwrap().digest.as_deref(), Some(taskrec::approval_digest(&redrafted).as_str()));
    let last = engine.seen().pop().unwrap();
    assert!(matches!(&last, Seen::Plan { resume, refusal: Some(n), .. } if *resume == sid && n == "shorter plan"), "{last:?}");

    approve(&path);
    edit(&path, |r| r.plan.push_str("\nextra"));
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let reset = load(&path);
    assert_eq!((reset.state, reset.approval.as_ref().unwrap().status.as_str()), (State::Defined, "pending"));
    assert_eq!(reset.approval.as_ref().unwrap().digest.as_deref(), Some(taskrec::approval_digest(&reset).as_str()));
    assert!(reset.waiting && reset.log.contains("approval digest mismatch, reset to pending"), "{}", reset.log);
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    assert_eq!(load(&path).log.matches("approval digest mismatch").count(), 1);
    assert_eq!(engine.count("execute"), 0);
}

#[test]
fn execute_logs_tool_use_and_the_validator_sets_the_verdict() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let exec = ScriptedExec::done("wrote greet.md")
        .tool("Read", json!({"file_path": root.join("notes.md").to_string_lossy()}))
        .tool("Bash", json!({"command": "ls -la"}))
        .write("greet.md", "hi");
    let engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(exec).with_cost(0.1);
    save_one(&ws, "split");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let started = load(&path);
    assert_eq!((started.state, started.step), (State::InProgress, Some(Step::Execute)));
    assert!(started.verdict.is_none());
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let executed = load(&path);
    assert_eq!((executed.state, executed.step), (State::InProgress, Some(Step::Evaluate)));
    assert!(executed.verdict.is_none());
    for line in ["execute: Read notes.md", "execute: Bash ls -la", "execute: Write greet.md", "execute: done: wrote greet.md"] {
        assert!(executed.log.contains(line), "{line}: {}", executed.log);
    }
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    let Some(Seen::Execute { allowed, read_only, resume, .. }) = engine.seen().pop() else { panic!() };
    assert_eq!((allowed, read_only, resume), (vec!["Read".into(), "Glob".into(), "Grep".into()], false, None));
    assert!(executed.sessions.execute.is_some());
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let judged = load(&path);
    assert_eq!((judged.state, judged.verdict.as_ref().unwrap().result.as_str()), (State::Succeeded, "pass"));
    assert_eq!(judged.verdict.as_ref().unwrap().items[0].id, "a1");
    assert!(judged.sessions.evaluate.is_some() && judged.sessions.evaluate != judged.sessions.execute);
    let Some(Seen::Evaluate { summary, log }) = engine.seen().pop() else { panic!() };
    assert!(summary == "wrote greet.md" && log.contains("execute: Write greet.md"), "{summary} {log}");
    assert!((judged.cost_usd - 0.4).abs() < 1e-9, "{}", judged.cost_usd);
}

#[test]
fn failed_check_does_not_succeed() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new()
        .define(defined())
        .plan(plan(PLAN, &["Write"], ("test", &["-f", "greet.md"])))
        .exec(ScriptedExec::done("did nothing"));
    let settings = fabric.join("settings.toml");
    std::fs::write(&settings, format!("replan_on_fail = false\n{}", std::fs::read_to_string(&settings).unwrap())).unwrap();
    save_one(&ws, "fail check");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.verdict.as_ref().unwrap().result.as_str()), (State::Failed, "fail"));
    assert!(!rec.log.contains("replanning") && engine.count("plan") == 1, "{}", rec.log);
    let node = ws.get_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    assert_eq!(node.items[0].phase, Phase::Failed);
}

#[test]
fn end_to_end_greet_file() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    assert_eq!((rec.state, rec.verdict.as_ref().unwrap().result.as_str()), (State::Succeeded, "pass"));
    assert!(rec.step.is_none() && rec.log.contains("learn: nothing new") && rec.log.contains("notify: result succeeded"));
    assert_eq!(std::fs::read_dir(fabric.join("registry")).map(|rd| rd.flatten().count()).unwrap_or(0), 0);
    let node = ws.get_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    assert_eq!(node.items[0].phase, Phase::Done);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(load(&path).state, State::Succeeded);
}

#[test]
fn a_question_mid_run_parks_and_the_answer_resumes_the_executor_session() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(ScriptedExec::ask("Which greeting?")).exec(writes_greet());
    save_one(&ws, "ask mid run");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let parked = load(&path);
    assert_eq!((parked.state, parked.step), (State::InProgress, Some(Step::Execute)));
    assert!(parked.waiting && parked.question.as_ref().unwrap().text == "Which greeting?");
    assert!(parked.log.contains("execute: asked q1"));
    let sid = parked.sessions.execute.clone().unwrap();
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(engine.count("execute"), 1, "a parked run must not start Claude Code");
    answer(&path, "say hi please");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.answers.len()), (State::Succeeded, 1));
    let execs: Vec<Seen> = engine.seen().into_iter().filter(|s| matches!(s, Seen::Execute { .. })).collect();
    let Seen::Execute { resume, answers, .. } = &execs[1] else { panic!() };
    assert_eq!(resume.as_deref(), Some(sid.as_str()));
    assert!(answers.contains("say hi please"));
}

#[test]
fn a_permission_denial_becomes_a_question_and_allow_adds_the_rule_and_resumes() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new()
        .define(defined())
        .plan(greet_plan())
        .exec(ScriptedExec::denied("Bash", json!({"command": "npm install left-pad"})))
        .exec(writes_greet());
    save_one(&ws, "deny");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let parked = load(&path);
    assert!(parked.waiting);
    let q = parked.question.as_ref().unwrap();
    assert!(
        q.text.contains("not allowed to run `npm install left-pad`")
            && q.text.contains("Reply allow to grant `Bash(npm install left-pad)`, or say what to do instead."),
        "{}",
        q.text
    );
    assert!(
        parked.log.contains("execute: denied Bash(npm install left-pad)\n") && parked.log.contains("execute: asked q1"),
        "{}",
        parked.log
    );
    let digest = parked.approval.as_ref().unwrap().digest.clone();
    answer(&path, " Allow ");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded, "{}", rec.log);
    let meta = rec.plan_meta.as_ref().unwrap();
    assert_eq!(meta.tools, ["Write", "Bash(npm install left-pad)"]);
    assert!(meta.requires.tools.contains(&"Bash(npm install left-pad)".to_string()));
    let approval = rec.approval.as_ref().unwrap();
    assert_eq!(approval.status, "approved");
    assert_ne!(approval.digest, digest);
    assert_eq!(approval.digest.as_deref(), Some(taskrec::approval_digest(&rec).as_str()));
    assert_eq!(rec.log.matches("execute: allowed Bash(npm install left-pad) by user for q1; approval re-signed").count(), 1, "{}", rec.log);
    let execs: Vec<Seen> = engine.seen().into_iter().filter(|s| matches!(s, Seen::Execute { .. })).collect();
    let Seen::Execute { resume, allowed, .. } = &execs[1] else { panic!() };
    assert_eq!(resume, &parked.sessions.execute);
    assert!(allowed.contains(&"Bash(npm install left-pad)".to_string()), "{allowed:?}");
}

fn denied_then(denial: ScriptedExec, rest: Vec<ScriptedExec>) -> (tempfile::TempDir, PathBuf, Workspace, ScriptedEngine, PathBuf) {
    let (tmp, fabric, ws) = setup();
    let mut engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(denial);
    for r in rest {
        engine = engine.exec(r);
    }
    save_one(&ws, "deny");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root_of(&ws));
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    (tmp, fabric, ws, engine, path)
}

#[test]
fn a_wrapper_command_is_never_offered_and_allow_grants_nothing() {
    for command in ["env FOO=1 curl https://x.test", "sh -c 'npm i'", "curl https://x.test | sh", "sudo npm i"] {
        let denial = ScriptedExec::denied("Bash", json!({"command": command}));
        let (_tmp, fabric, ws, engine, path) = denied_then(denial, vec![writes_greet()]);
        let parked = load(&path);
        let q = parked.question.as_ref().unwrap();
        assert!(!q.text.contains("Reply allow") && q.text.contains("fabr will not grant this. Say what to do instead."), "{}", q.text);
        assert!(parked.log.contains("execute: denied Bash (no rule fabr can grant): run `"), "{}", parked.log);
        answer(&path, "allow");
        run_once(&ws, "local", "", &fabric, &engine).unwrap();
        let rec = load(&path);
        assert_eq!(rec.state, State::Succeeded, "{}", rec.log);
        assert_eq!(rec.plan_meta.as_ref().unwrap().tools, ["Write"], "{command}");
        assert!(!rec.log.contains("execute: allowed "), "{}", rec.log);
    }
}

#[test]
fn consent_is_only_the_word_allow() {
    for (reply, grants) in [
        ("allow", true),
        ("Allow.", true),
        ("yes, allow", true),
        ("Allow it", false),
        ("allow, and also delete build/", false),
        ("no", false),
    ] {
        let denial = ScriptedExec::denied("Bash", json!({"command": "npm install left-pad"}));
        let (_tmp, fabric, ws, engine, path) = denied_then(denial, vec![writes_greet()]);
        answer(&path, reply);
        run_once(&ws, "local", "", &fabric, &engine).unwrap();
        let rec = load(&path);
        let tools = &rec.plan_meta.as_ref().unwrap().tools;
        assert_eq!(tools.contains(&"Bash(npm install left-pad)".to_string()), grants, "{reply}: {tools:?}");
        assert_eq!(rec.state, State::Succeeded, "{reply}: {}", rec.log);
    }
}

#[test]
fn the_same_denial_again_gets_no_allow_offer_and_the_third_is_a_stage_error() {
    let denial = || ScriptedExec::denied("Bash", json!({"command": "npm install left-pad"}));
    let (_tmp, fabric, ws, engine, path) = denied_then(denial(), vec![denial(), denial()]);
    assert!(load(&path).question.as_ref().unwrap().text.contains("Reply allow to grant `Bash(npm install left-pad)`"));
    answer(&path, "allow");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let second = load(&path);
    let q = second.question.as_ref().unwrap();
    assert!(!q.text.contains("Reply allow") && q.text.contains("already allowed"), "{}", q.text);
    assert_eq!(second.log.matches("by user for q").count(), 1, "{}", second.log);
    answer(&path, "allow");
    let err = run_once(&ws, "local", "", &fabric, &engine).unwrap_err();
    assert!(err.message().contains("denied the same thing 3 times in a row"), "{}", err.message());
    let rec = load(&path);
    assert_eq!(engine.count("execute"), 3);
    assert!(rec.question.is_none() && rec.log.contains("execute: error 1: Claude Code was denied the same thing 3 times"), "{}", rec.log);
    assert_eq!(rec.plan_meta.as_ref().unwrap().tools, ["Write", "Bash(npm install left-pad)"]);
}

#[test]
fn a_digest_tampered_while_parked_is_caught_before_the_next_execute_run() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(ScriptedExec::ask("Which greeting?")).exec(writes_greet());
    save_one(&ws, "tamper");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(engine.count("execute"), 1);
    edit(&path, |r| r.plan_meta.as_mut().unwrap().tools.push("Bash(curl https://evil.test)".into()));
    answer(&path, "hi");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(engine.count("execute"), 1, "{}", rec.log);
    assert_eq!((rec.state, rec.step, rec.waiting), (State::Defined, Some(Step::Approve), true));
    let approval = rec.approval.as_ref().unwrap();
    assert_eq!((approval.status.as_str(), approval.digest.as_deref()), ("pending", Some(taskrec::approval_digest(&rec).as_str())));
    assert!(rec.log.contains("approve: approval digest mismatch before execute, reset to pending"), "{}", rec.log);
}

#[test]
fn a_record_that_arrives_in_progress_without_approval_does_not_execute() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "forged");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.plan = PLAN.into();
        r.plan_meta = Some(taskrec::PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec!["Bash(rm -rf ~)".into()],
            assumptions: vec![],
            checks: vec![],
            requires: Default::default(),
        });
    });
    let engine = ScriptedEngine::new().exec(writes_greet());
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(engine.count("execute"), 0);
    assert_eq!((rec.state, rec.step, rec.waiting), (State::Defined, Some(Step::Approve), true));
    assert_eq!(rec.approval.as_ref().unwrap().status, "pending");
    assert!(rec.log.contains("approve: approval is missing before execute, reset to pending"), "{}", rec.log);
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.waiting = false;
        r.approval.as_mut().unwrap().by = Some("auto".into());
        r.plan_meta.as_mut().unwrap().reversibility = Some("irreversible".into());
        common::sign(r);
        r.approval.as_mut().unwrap().by = Some("auto".into());
    });
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    assert_eq!(engine.count("execute"), 0);
    assert!(load(&path).log.contains("auto approval is not allowed for an irreversible plan before execute"));
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.waiting = false;
        r.plan_meta.as_mut().unwrap().reversibility = Some("reversible".into());
        r.sessions.execute = Some("--dangerously-skip-permissions".into());
        common::sign(r);
    });
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let Some(Seen::Execute { resume, .. }) = engine.seen().pop() else { panic!("{}", load(&path).log) };
    assert_eq!(resume, None);
}

#[test]
fn the_task_spend_cap_stops_the_task_before_the_next_run() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let settings = fabric.join("settings.toml");
    let text = std::fs::read_to_string(&settings).unwrap();
    std::fs::write(&settings, format!("max_task_usd = 0.5\n{text}")).unwrap();
    let engine = greet().with_cost(0.3);
    save_one(&ws, "capped");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    assert_eq!(load(&path).cost_usd, 0.6);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(engine.count("execute"), 0);
    assert_eq!((rec.state, rec.step), (State::Failed, None));
    assert!(rec.log.contains("execute: stopped: task spend cap reached ($0.60 of $0.50)"), "{}", rec.log);
}

#[test]
fn a_task_at_its_spend_cap_succeeds_without_proposing() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let settings = fabric.join("settings.toml");
    std::fs::write(&settings, format!("max_task_usd = 1.2\n{}", std::fs::read_to_string(&settings).unwrap())).unwrap();
    let engine = greet().with_cost(0.3);
    save_one(&ws, "capped proposal");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Succeeded, None), "{}", rec.log);
    assert!(rec.log.contains("learn: no proposal (task spend cap reached)") && engine.count("propose") == 0, "{}", rec.log);
}

#[test]
fn a_failed_check_over_the_task_spend_cap_fails_without_a_replan() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let settings = fabric.join("settings.toml");
    std::fs::write(&settings, format!("max_task_usd = 1.0\n{}", std::fs::read_to_string(&settings).unwrap())).unwrap();
    let engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(ScriptedExec::done("did nothing")).with_cost(0.3);
    save_one(&ws, "capped replan");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.verdict.as_ref().unwrap().result.as_str()), (State::Failed, "fail"), "{}", rec.log);
    assert!(!rec.log.contains("replanning") && engine.count("plan") == 1, "{}", rec.log);
}

#[test]
fn a_live_chat_on_the_session_holds_the_run_and_a_stale_marker_is_removed() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "chat");
    let sid = "0b8f6c1e-3a4d-4f5e-9a6b-7c8d9e0f1a2b";
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.plan = PLAN.into();
        r.sessions.execute = Some(sid.into());
        common::sign(r);
    });
    let lock = root.join(".fabric-space").join("chat").join(format!("{sid}.lock"));
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    std::fs::write(&lock, format!("{}\n", std::process::id())).unwrap();
    let engine = ScriptedEngine::new().exec(writes_greet());
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(engine.count("execute"), 0);
    assert_eq!(rec.log.matches("execute: waiting for a chat on this session to finish").count(), 1, "{}", rec.log);
    std::fs::write(&lock, "2147483000\n").unwrap();
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    assert_eq!(engine.count("execute"), 1);
    assert!(!lock.exists());
    let Some(Seen::Execute { resume, .. }) = engine.seen().pop() else { panic!() };
    assert_eq!(resume.as_deref(), Some(sid));
}

#[test]
fn another_answer_to_a_denial_adds_no_rule() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new()
        .define(defined())
        .plan(greet_plan())
        .exec(ScriptedExec::denied("WebFetch", json!({"url": "https://example.com/a"})))
        .exec(writes_greet());
    save_one(&ws, "deny");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert!(load(&path).log.contains("execute: denied WebFetch(domain:example.com)"));
    answer(&path, "no, write it from memory");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded);
    assert_eq!(rec.plan_meta.as_ref().unwrap().tools, ["Write"]);
    assert!(!rec.log.contains("allowed "));
}

struct CancelsMidRun {
    path: PathBuf,
}

impl Engine for CancelsMidRun {
    fn define(&self, _: &DefineInput, _: &mut dyn Watch) -> Run<DefineOut> {
        unreachable!()
    }

    fn plan(&self, _: &PlanInput, _: &mut dyn Watch) -> Run<PlanOut> {
        unreachable!()
    }

    fn execute(&self, input: &ExecuteInput, watch: &mut dyn Watch) -> Run<ExecuteOut> {
        watch.session("11111111-1111-4111-8111-111111111111");
        watch.tool(&ToolUse { name: "Write".into(), input: json!({"file_path": "first.md"}) });
        std::fs::write(input.cwd.join("first.md"), "x").unwrap();
        edit(&self.path, |r| r.requests.cancel = true);
        assert!(watch.stop());
        Run { session_id: None, cost_usd: 0.0, usage: Usage::default(), denials: vec![], end: End::Stopped }
    }

    fn evaluate(&self, _: &EvaluateInput, _: &mut dyn Watch) -> Run<EvaluateOut> {
        unreachable!()
    }

    fn propose(&self, _: &ProposeInput, _: &mut dyn Watch) -> Run<ProposeOut> {
        unreachable!()
    }
}

#[test]
fn a_cancel_during_a_run_stops_it_and_fails_the_task() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "job");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.plan = "1. write".into();
        common::sign(r);
    });
    advance(&root, &path, &fabric, 10, &CancelsMidRun { path: path.clone() }).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Failed, None));
    assert!(!rec.requests.cancel && rec.verdict.is_none());
    assert!(rec.log.contains("execute: Write first.md") && rec.log.contains("execute: run stopped"), "{}", rec.log);
    assert!(rec.log.contains("execute: cancelled by user"));
    assert_eq!(rec.sessions.execute.as_deref(), Some("11111111-1111-4111-8111-111111111111"));
    assert!(root.join("first.md").exists());
}

#[test]
fn three_errors_on_one_step_fail_the_task_with_the_reason_in_the_log() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "job");
    let engine = ScriptedEngine::new();
    for _ in 0..3 {
        engine.push_define(Err("Claude Code said no, key sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa".into()));
    }
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    for n in 1..=3 {
        let err = advance(&root, &path, &fabric, 10, &engine);
        let rec = load(&path);
        if n < 3 {
            assert!(err.is_err());
            assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
            assert!(rec.log.contains(&format!("define: error {n}:")), "{}", rec.log);
        } else {
            assert!(err.is_ok());
            assert_eq!((rec.state, rec.step), (State::Failed, None));
            assert!(rec.verdict.is_none() && !rec.waiting);
            assert!(rec.log.contains("define: failed after 3 errors: Claude Code said no"), "{}", rec.log);
        }
        assert!(!rec.log.contains("FAKEFAKE"), "{}", rec.log);
    }
    let again = load(&path);
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    assert_eq!(load(&path).log, again.log);
}

#[test]
fn tool_lines_between_errors_do_not_reset_the_error_count() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "job");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.log.push_str("- 2026-10-30T00:00:00Z execute: error 1: boom\n- 2026-10-30T00:00:01Z execute: Edit a.md\n");
        common::sign(r);
    });
    let engine = ScriptedEngine::new();
    engine.push_exec(Err("boom again".into()));
    assert!(advance(&root, &path, &fabric, 10, &engine).is_err());
    assert!(load(&path).log.contains("execute: error 2: boom again"));
}

#[test]
fn a_gui_write_between_read_and_write_is_folded_in() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = save_one(&ws, "job");
    let engine = ScriptedEngine::new().define(asks("Which filename?"));
    for _ in 0..3 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
    }
    let mut mine = load(&path);
    let mut base = mine.clone();
    assert!(mine.question.is_some());
    let ans = taskrec::Answer { value: Some("greet.md".into()), replacement: None, granted: None, by: Some("gui".into()), at: None };
    edit(&path, |r| {
        r.requests.cancel = true;
        r.question.as_mut().unwrap().answer = Some(ans.clone());
        r.grants.granted = vec![taskrec::Grant { field: "address.zip".into(), scope: "task".into() }];
    });
    fabr_agent::state::append_log(&mut mine.log, "define", "stale stage wrote this");
    mine.spec = "new spec".into();
    mine.sessions.execute = Some("s".into());
    fabr_agent::store::update(&path, &mut mine, &mut base).unwrap();
    let disk = load(&path);
    assert!(disk.requests.cancel);
    assert_eq!(disk.question.as_ref().unwrap().answer.as_ref(), Some(&ans));
    assert_eq!(disk.grants.granted.len(), 1);
    assert_eq!((disk.spec.as_str(), disk.sessions.execute.as_deref()), ("new spec", Some("s")));
    assert!(disk.log.contains("stale stage wrote this"));
    assert_eq!(mine, disk);
}

#[test]
fn a_hung_check_is_killed_and_counted_as_a_stage_error() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    std::fs::write(root.join("greet.md"), "hi").unwrap();
    let path = save_one(&ws, "job");
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Evaluate);
        r.plan_meta = Some(taskrec::PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec![],
            assumptions: vec![],
            checks: vec![taskrec::Check { cmd: "tail".into(), args: vec!["-f".into(), "greet.md".into()] }],
            requires: Default::default(),
        });
    });
    let mut ctl = fabr_agent::Ctl::new(&fabric, 10);
    ctl.check_timeout_s = 1;
    let started = std::time::Instant::now();
    let err = fabr_agent::advance_with(&root, &path, &ctl, &ScriptedEngine::new()).unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(err.message().contains("timed out"), "{}", err.message());
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::InProgress, Some(Step::Evaluate)));
    assert!(rec.log.contains("evaluate: error 1: check timed out"), "{}", rec.log);
}

fn registry_step(fabric: &Path) -> Option<(String, String)> {
    let entry = std::fs::read_dir(fabric.join("registry"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "yaml"))?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    Some((doc["state"].as_str().unwrap().to_string(), doc["step"].as_str().unwrap().to_string()))
}

#[test]
fn the_registry_step_is_never_stale_through_evaluate_learn_and_notify() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    save_one(&ws, "track steps");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    assert_eq!(registry_step(&fabric), Some(("defined".into(), "approve".into())));
    approve(&path);
    let mut seen = Vec::new();
    for _ in 0..6 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
        seen.push(registry_step(&fabric));
    }
    let step = |s: &str, t: &str| Some((s.to_string(), t.to_string()));
    assert_eq!(
        seen,
        vec![
            step("in_progress", "execute"),
            step("in_progress", "evaluate"),
            step("succeeded", "learn"),
            step("succeeded", "notify"),
            None,
            None
        ]
    );
}

#[test]
fn define_sets_the_requested_grants() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut d = defined();
    d.grants = vec!["address.zip".into(), "address.zip".into(), " ".into(), "name.first".into()];
    let engine = ScriptedEngine::new().define(d).plan(greet_plan());
    save_one(&ws, "needs data");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!(rec.grants.requested, vec!["address.zip".to_string(), "name.first".to_string()]);
    assert!(rec.grants.granted.is_empty());
}

fn entry(fabric: &Path, path: &Path) -> Option<serde_yaml::Value> {
    let file = fabr_agent::registry_path(fabric, &load(path).id, &fabr_agent::task_path_key(path));
    serde_yaml::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

fn assert_mirrors(fabric: &Path, path: &Path, tick: &str) {
    let rec = load(path);
    let got = entry(fabric, path).unwrap_or_else(|| panic!("{tick}: no registry entry"));
    let text = |k: &str| got[k].as_str().map(str::to_string);
    assert_eq!(text("state").as_deref(), Some(fabr_agent::state::state_name(rec.state)), "{tick}");
    assert_eq!(text("step").as_deref(), rec.step.map(fabr_agent::state::step_name), "{tick}");
    assert_eq!(text("waiting").as_deref(), fabr_agent::state::waiting_on(&rec), "{tick}");
    assert!(text("activity").is_some(), "{tick}: activity missing");
}

#[test]
fn registry_entry_mirrors_the_record_from_gather_to_notify_and_cancel_removes_it() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut d = defined();
    d.assertions[0].text = "the folder exists".into();
    let engine = ScriptedEngine::new()
        .define(asks("Which filename?"))
        .define(d)
        .plan(plan(PLAN, &[], ("test", &["-d", "."])))
        .exec(ScriptedExec::done("looked"));
    let path = save_one(&ws, "observe me");
    let tick = |name: &str| {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
        name.to_string()
    };
    let t = tick("delegate");
    assert_eq!(load(&path).step, Some(Step::Gather), "{t}");
    for name in ["gather", "define-question"] {
        let t = tick(name);
        assert_mirrors(&fabric, &path, &t);
    }
    assert_eq!(entry(&fabric, &path).unwrap()["waiting"].as_str(), Some("question"));
    answer(&path, "greet.md");
    for name in ["define-answered", "plan"] {
        let t = tick(name);
        assert_mirrors(&fabric, &path, &t);
    }
    assert_eq!(entry(&fabric, &path).unwrap()["waiting"].as_str(), Some("approval"));
    approve(&path);
    for name in ["approved", "execute", "evaluate", "learn"] {
        let t = tick(name);
        assert_mirrors(&fabric, &path, &t);
    }
    tick("notify");
    let done = load(&path);
    assert_eq!((done.state, done.step), (State::Succeeded, None));
    assert!(entry(&fabric, &path).is_none());

    let engine = greet();
    let second = open_reminder(&root, "cancel me").unwrap();
    edit(&second, |r| r.requests.delegate = true);
    for _ in 0..4 {
        advance(&root, &second, &fabric, 10, &engine).unwrap();
    }
    approve(&second);
    advance(&root, &second, &fabric, 10, &engine).unwrap();
    assert_eq!(load(&second).step, Some(Step::Execute));
    assert_mirrors(&fabric, &second, "before cancel");
    edit(&second, |r| r.requests.cancel = true);
    advance(&root, &second, &fabric, 10, &engine).unwrap();
    let rec = load(&second);
    assert!(rec.state == State::Failed && rec.log.contains("execute: cancelled by user"));
    assert!(entry(&fabric, &second).is_none());
    assert_eq!(engine.count("execute"), 0);
}

fn drive_n(root: &Path, path: &Path, ctl: &fabr_agent::Ctl, engine: &ScriptedEngine, n: usize) {
    for _ in 0..n {
        let _ = fabr_agent::advance_with(root, path, ctl, engine);
    }
}

#[test]
fn a_read_only_machine_plans_and_auto_approves_then_parks_at_execute_and_a_capable_one_executes() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    let path = save_one(&ws, "greet");
    let mut ro = fabr_agent::Ctl::new(&fabric, 10);
    ro.harness.mode = fabr_core::setup::ExecuteMode::ReadOnly;
    ro.harness.auto_mode = true;
    drive_n(&root, &path, &ro, &engine, 10);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!(rec.approval.as_ref().map(|a| (a.status.as_str(), a.by.as_deref())), Some(("approved", Some("auto"))));
    assert_eq!(rec.needs.as_deref(), Some("harness: Write, checks: test"));
    assert_eq!(rec.log.matches("execute: needs harness: Write, checks: test").count(), 1, "{}", rec.log);
    assert_eq!(Phase::of(&rec, "2026-10-25T12:00"), Phase::NeedsHarness);
    assert_eq!(engine.count("execute"), 0);
    let full = fabr_agent::Ctl::new(&fabric, 10);
    drive_n(&root, &path, &full, &engine, 12);
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded, "{}", rec.log);
    assert!(rec.needs.is_none());
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
}

#[test]
fn a_read_only_plan_runs_with_a_read_only_executor() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut d = defined();
    d.assertions[0].text = "the folder was looked at".into();
    let look = PlanOut { checks: vec![], ..plan(PLAN, &["Read"], ("true", &[])) };
    let engine = ScriptedEngine::new()
        .define(d)
        .plan(look)
        .exec(ScriptedExec::done("looked"))
        .verdict(EvaluateOut { items: vec![JudgedItem { id: "a1".into(), verdict: Judgement::Pass, evidence: "read it".into() }] });
    let path = save_one(&ws, "look");
    let mut ro = fabr_agent::Ctl::new(&fabric, 10);
    ro.harness.mode = fabr_core::setup::ExecuteMode::ReadOnly;
    drive_n(&root, &path, &ro, &engine, 4);
    approve(&path);
    drive_n(&root, &path, &ro, &engine, 2);
    let execs: Vec<Seen> = engine.seen().into_iter().filter(|s| matches!(s, Seen::Execute { .. })).collect();
    assert!(matches!(&execs[..], [Seen::Execute { read_only: true, .. }]), "{execs:?} {}", load(&path).log);
    drive_n(&root, &path, &ro, &engine, 3);
    assert_eq!(load(&path).state, State::Succeeded);
}

#[test]
fn human_sections_are_not_regenerated_and_redo_steps_back() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    let path = save_one(&ws, "greet");
    let ctl = fabr_agent::Ctl::new(&fabric, 10);
    drive_n(&root, &path, &ctl, &engine, 2);
    assert_eq!(load(&path).step, Some(Step::Define));
    edit(&path, |r| {
        r.spec = "my own spec".into();
        r.success = "my own success".into();
        r.plan = "my own plan".into();
        r.by.spec = Some(taskrec::Author::Human);
        r.by.success = Some(taskrec::Author::Human);
        r.by.plan = Some(taskrec::Author::Human);
        r.assertions = vec![taskrec::Assertion { id: "h1".into(), text: "greet.md contains hi".into() }];
    });
    drive_n(&root, &path, &ctl, &engine, 3);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!((rec.spec.as_str(), rec.success.as_str(), rec.plan.as_str()), ("my own spec", "my own success", "my own plan"));
    assert_eq!((rec.assertions[0].id.as_str(), rec.by.plan), ("h1", Some(taskrec::Author::Human)));
    assert_eq!(rec.plan_meta.as_ref().unwrap().requires.checks, vec!["test".to_string()]);
    assert_eq!(engine.count("define"), 0);
    edit(&path, |r| r.requests.redo = Some(taskrec::RedoStage::Plan));
    drive_n(&root, &path, &ctl, &engine, 1);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Plan)));
    assert!(rec.plan.is_empty() && rec.approval.is_none() && rec.plan_meta.is_none() && rec.requests.redo.is_none());
    assert!(rec.sessions.plan.is_some() && rec.log.contains("redo: plan by user"), "{}", rec.log);
    edit(&path, |r| r.requests.redo = Some(taskrec::RedoStage::Define));
    drive_n(&root, &path, &ctl, &engine, 1);
    let rec = load(&path);
    assert!(rec.sessions.plan.is_none() && rec.step == Some(Step::Define));
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.requests.redo = Some(taskrec::RedoStage::Define);
    });
    drive_n(&root, &path, &ctl, &engine, 1);
    let rec = load(&path);
    assert!(rec.requests.redo.is_none() && rec.log.contains("ignored request redo in state in_progress"), "{}", rec.log);
}

#[test]
fn a_failed_check_replans_once_and_a_second_failure_fails_the_task() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut d = defined();
    d.assertions.push(DraftAssertion { id: "a2".into(), text: "greet.md is the only file written".into() });
    d.assertions.push(DraftAssertion { id: "a3".into(), text: "nothing else changed".into() });
    let j = |id: &str, verdict, evidence: &str| JudgedItem { id: id.into(), verdict, evidence: evidence.into() };
    let verdict = EvaluateOut {
        items: vec![j("a1", Judgement::Pass, "ok"), j("a2", Judgement::Fail, "also wrote x.txt"), j("a3", Judgement::Unchecked, "?")],
    };
    let revised = plan(&format!("{PLAN}\n2. Write nothing else"), &["Write"], ("test", &["-f", "greet.md"]));
    let engine = ScriptedEngine::new()
        .define(d)
        .plan(greet_plan())
        .plan(revised)
        .exec(writes_greet())
        .exec(writes_greet())
        .verdict(verdict.clone())
        .verdict(verdict);
    save_one(&ws, "verdict");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    for _ in 0..3 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
    }
    let rec = load(&path);
    assert_eq!((rec.state, rec.step, rec.waiting, rec.approval.is_none()), (State::Task, Some(Step::Plan), false, true), "{}", rec.log);
    assert_eq!(registry_step(&fabric), Some(("task".into(), "plan".into())));
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let rec = load(&path);
    assert_eq!((rec.state, rec.step, rec.waiting), (State::Defined, Some(Step::Approve), true), "{}", rec.log);
    assert_eq!(rec.approval.as_ref().unwrap().status, "pending");
    assert_eq!(rec.verdict.as_ref().unwrap().result, "fail");
    assert!(rec.log.contains("evaluate: replanning after failed check (a2, a3)") && rec.log.contains("plan: redrafted after failed check"));
    let Some(Seen::Plan { resume, failed: Some(note), .. }) = engine.seen().into_iter().rfind(|s| matches!(s, Seen::Plan { .. })) else {
        panic!()
    };
    assert_eq!(resume, rec.sessions.plan);
    assert_eq!(note, "- a2: also wrote x.txt\n- a3: unchecked: ?");
    let first_exec = rec.sessions.execute.clone();
    approve(&path);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&path);
    let execs: Vec<Seen> = engine.seen().into_iter().filter(|s| matches!(s, Seen::Execute { .. })).collect();
    let Seen::Execute { resume, failed: Some(note), .. } = &execs[1] else { panic!("{execs:?}") };
    assert!(*resume == first_exec && note.contains("a2: also wrote x.txt"), "{execs:?}");
    assert_eq!((engine.count("plan"), engine.count("execute"), engine.count("evaluate")), (2, 2, 2));
    let v = rec.verdict.as_ref().unwrap();
    assert_eq!((rec.state, v.result.as_str()), (State::Failed, "fail"));
    assert_eq!(rec.log.matches("replanning after failed check").count(), 1);
    assert_eq!((v.items[0].pass, v.items[1].pass, v.items[1].evidence.as_str()), (true, false, "also wrote x.txt"));
    assert_eq!((v.items[2].pass, v.items[2].evidence.as_str()), (false, "unchecked: ?"));
}

#[test]
fn a_check_on_a_file_that_is_not_in_the_folder_or_created_is_dropped() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut p = greet_plan();
    p.checks.push(DraftCheck { cmd: "grep".into(), args: vec!["-q".into(), "done".into(), "tasks.md".into()] });
    let engine = ScriptedEngine::new().define(defined()).plan(p);
    save_one(&ws, "ground");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let rec = load(&record_path(&root));
    let checks = &rec.plan_meta.as_ref().unwrap().checks;
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].args, vec!["-f".to_string(), "greet.md".to_string()]);
    assert!(rec.log.contains("plan: dropped check grep -q done tasks.md: unknown file"), "{}", rec.log);
}

#[test]
fn more_than_three_assertions_is_a_stage_error() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let mut d = defined();
    d.assertions = (1..=4).map(|n| DraftAssertion { id: format!("a{n}"), text: format!("claim {n}") }).collect();
    let engine = ScriptedEngine::new().define(d);
    save_one(&ws, "too many");
    let err = run_once(&ws, "local", "", &fabric, &engine).unwrap_err();
    assert!(err.message().contains("4 assertions; 1 to 3 are allowed"), "{}", err.message());
    assert!(load(&record_path(&root)).log.contains("define: error 1"));
}

#[test]
fn a_reminder_without_a_delegate_request_is_not_touched() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let path = open_reminder(&root, "idle").unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let engine = greet();
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

fn fixture_markdown() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for ent in std::fs::read_dir(dir).unwrap().flatten() {
            let path = ent.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "md") && path.components().any(|c| c.as_os_str() == "tasks") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"), &mut out);
    out.sort();
    out
}

fn waiting_on_question(rec: &TaskRecord) -> bool {
    rec.waiting && rec.question.as_ref().is_some_and(|q| q.answer.is_none())
}

fn waiting_on_approval(rec: &TaskRecord) -> bool {
    rec.state == State::Defined && rec.approval.as_ref().is_none_or(|a| a.status != "approved" && a.status != "refused")
}

#[test]
fn ten_fixtures_one_stage_matches_fabric_updated_at() {
    let fixtures = fixture_markdown();
    assert_eq!(fixtures.len(), 12, "expected twelve fixture records");
    for fixture in fixtures {
        let text = std::fs::read_to_string(&fixture).unwrap();
        let original = taskrec::parse(&text).unwrap_or_else(|e| panic!("{}: {}", fixture.display(), e.message()));
        assert!(text.starts_with("---\nschema: 4\n"), "{}", fixture.display());
        assert_eq!((original.schema, original.pri), (7, 0), "{}", fixture.display());
        assert!(original.source.is_none(), "{}", fixture.display());
        assert_eq!(fabric::updated_at(&text), original.updated_at, "{}", fixture.display());

        let (_tmp, fabric_space, ws) = setup();
        let root = root_of(&ws);
        let dest = root.join(".fabric-space").join("tasks").join(format!("{}.md", original.id));
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        taskrec::write_record(&dest, &original, None).unwrap();
        let mut before = load(&dest);
        if before.state == State::Reminder {
            before.requests.delegate = true;
            let expected = before.updated_at.clone();
            taskrec::write_record(&dest, &before, Some(&expected)).unwrap();
            before = load(&dest);
        }
        let engine = ScriptedEngine::new().define(defined()).plan(greet_plan()).exec(ScriptedExec::done("ok"));
        if !waiting_on_question(&before) && !waiting_on_approval(&before) {
            advance(&root, &dest, &fabric_space, 10, &engine).unwrap();
        }
        let staged = load(&dest);
        let expected = staged.updated_at.clone();
        taskrec::write_record(&dest, &staged, Some(&expected)).unwrap();
        let disk = std::fs::read_to_string(&dest).unwrap();
        let again = taskrec::parse(&disk).unwrap();
        assert_eq!(fabric::updated_at(&disk), again.updated_at, "{}", fixture.display());
        assert_eq!((again.state, again.step), (staged.state, staged.step), "{}", fixture.display());

        if waiting_on_question(&before) {
            assert!(again.waiting, "{}", fixture.display());
            assert_eq!((again.state, again.step), (before.state, before.step), "{}", fixture.display());
            assert!(again.question.as_ref().unwrap().answer.is_none(), "{}", fixture.display());
            assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
        } else if waiting_on_approval(&before) {
            assert_eq!((again.state, again.step), (State::Defined, Some(Step::Approve)), "{}", fixture.display());
            assert!(again.waiting, "{}", fixture.display());
            assert_eq!(again.approval.as_ref().unwrap().status, "pending", "{}", fixture.display());
            assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
        } else {
            match (before.state, before.step) {
                (State::Reminder, _) => {
                    assert_eq!((again.state, again.step), (State::Task, Some(Step::Gather)), "{}", fixture.display());
                    assert!(!again.waiting, "{}", fixture.display());
                }
                (State::Task, Some(Step::Define)) => {
                    assert_eq!((again.state, again.step), (State::Task, Some(Step::Plan)), "{}", fixture.display());
                    assert!(!again.waiting, "{}", fixture.display());
                }
                (State::InProgress, Some(Step::Execute)) if before.paused => {
                    assert_eq!((again.state, again.step), (before.state, before.step), "{}", fixture.display());
                    assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
                }
                (State::InProgress, Some(Step::Execute)) => {
                    assert_eq!(again.state, State::InProgress, "{}", fixture.display());
                    let unsupported = before.plan_meta.as_ref().is_some_and(|m| m.tools.iter().any(|t| t == "browser"));
                    assert_eq!(again.needs.is_some(), unsupported, "{}", fixture.display());
                    let want = if unsupported { Step::Execute } else { Step::Evaluate };
                    assert_eq!(again.step, Some(want), "{}", fixture.display());
                }
                (State::Succeeded | State::Failed, _) => {
                    assert_eq!((again.state, again.step), (before.state, before.step), "{}", fixture.display());
                    assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
                }
                _ => panic!("no scripted stage for {} {:?}", fixture.display(), (before.state, before.step)),
            }
            if !before.paused && !matches!(before.state, State::Succeeded | State::Failed) {
                assert_ne!(again.updated_at, before.updated_at, "{}", fixture.display());
            }
        }
    }
}

fn gui_write(path: &Path, rec: &TaskRecord, expected: &str) {
    fabric::write(&FabricWriteReq {
        path: path.to_string_lossy().into_owned(),
        text: taskrec::render(rec),
        expected_updated_at: expected.into(),
    })
    .unwrap();
}

#[test]
fn gui_approval_does_not_write_state_and_execute_waits_for_it() {
    let (_tmp, fabric_space, ws) = setup();
    let root = root_of(&ws);
    let engine = greet();
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric_space, &engine).unwrap();
    let path = record_path(&root);
    let drafted = load(&path);
    assert_eq!((drafted.state, drafted.step), (State::Defined, Some(Step::Approve)));
    assert!(drafted.waiting && drafted.source.is_none());
    let mut moved = drafted.clone();
    moved.state = State::InProgress;
    let refused = fabric::write(&FabricWriteReq {
        path: path.to_string_lossy().into_owned(),
        text: taskrec::render(&moved),
        expected_updated_at: drafted.updated_at.clone(),
    })
    .unwrap_err();
    assert!(refused.message().contains("state"));
    let mut approved = drafted.clone();
    let approval = approved.approval.as_mut().unwrap();
    approval.status = "approved".into();
    approval.by = Some("gui".into());
    approval.at = Some("2026-10-24T19:00:00Z".into());
    approval.digest = None;
    approved.updated_at = "2026-10-24T19:00:00Z".into();
    gui_write(&path, &approved, &drafted.updated_at);
    let on_disk = load(&path);
    assert_eq!((on_disk.state, on_disk.approval.as_ref().unwrap().status.as_str()), (State::Defined, "approved"));
    assert_eq!(on_disk.approval.as_ref().unwrap().digest, drafted.approval.as_ref().unwrap().digest);
    assert_eq!(on_disk.sessions, drafted.sessions);
    approval_allows_execute(&on_disk).unwrap();
    assert_eq!(engine.count("execute"), 0);
    run_once(&ws, "local", "", &fabric_space, &engine).unwrap();
    assert_eq!(engine.count("execute"), 1);
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    assert_eq!(load(&path).state, State::Succeeded);
}

#[test]
fn scripted_engine_reports_cost_and_sessions_without_a_watch() {
    let engine = ScriptedEngine::new().define(defined()).with_cost(0.02);
    let tmp = tempfile::tempdir().unwrap();
    let run = Engine::define(&engine, &DefineInput { cwd: tmp.path(), resume: None, title: "t", context: "", answers: "" }, &mut NoWatch);
    assert!(matches!(run.end, End::Done(_)) && run.cost_usd == 0.02 && run.session_id.is_some());
}

fn failed_a1() -> EvaluateOut {
    EvaluateOut { items: vec![JudgedItem { id: "a1".into(), verdict: Judgement::Fail, evidence: "greet.md is empty".into() }] }
}

fn replan_with_a_human_plan(second: PlanOut) -> (tempfile::TempDir, ScriptedEngine, TaskRecord) {
    let (tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new()
        .define(defined())
        .plan(greet_plan())
        .plan(second)
        .exec(ScriptedExec::done("wrote").write("greet.md", ""))
        .verdict(failed_a1());
    save_one(&ws, "greet");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    edit(&path, |r| {
        r.plan = format!("{PLAN}\n2. Mine");
        r.by.plan = Some(taskrec::Author::Human);
        common::sign(r);
    });
    for _ in 0..4 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
    }
    (tmp, engine, load(&path))
}

#[test]
fn a_failed_check_on_a_human_plan_still_asks_the_planner_and_says_when_nothing_changed() {
    let (_tmp, engine, rec) = replan_with_a_human_plan(plan("", &["Write"], ("test", &["-f", "greet.md"])));
    assert_eq!((rec.state, rec.step, rec.approval.as_ref().unwrap().status.as_str()), (State::Defined, Some(Step::Approve), "pending"));
    assert!(rec.log.contains("plan: unchanged after failed check") && !rec.log.contains("redrafted"), "{}", rec.log);
    assert_eq!((rec.plan.as_str(), rec.by.plan), (format!("{PLAN}\n2. Mine").as_str(), Some(taskrec::Author::Human)));
    let Some(Seen::Plan { failed, human_plan, .. }) = engine.seen().into_iter().rfind(|s| matches!(s, Seen::Plan { .. })) else { panic!() };
    assert_eq!((failed.as_deref(), human_plan.as_deref()), (Some("- a1: greet.md is empty"), Some(rec.plan.as_str())));
    assert_eq!(engine.count("plan"), 2);

    let revised = plan(&format!("{PLAN}\n2. Put hi in it"), &["Write"], ("test", &["-s", "greet.md"]));
    let (_tmp, _, rec) = replan_with_a_human_plan(revised);
    assert!(rec.log.contains("plan: redrafted after failed check"), "{}", rec.log);
    assert!(rec.plan.ends_with("2. Put hi in it") && rec.by.plan == Some(taskrec::Author::Agent), "{}", rec.plan);
}

#[test]
fn the_revised_plan_reaches_the_executor_until_a_run_on_it_produces_a_turn() {
    let (_tmp, fabric, ws) = setup();
    let root = root_of(&ws);
    let engine = ScriptedEngine::new()
        .define(defined())
        .plan(greet_plan())
        .plan(plan(&format!("{PLAN}\n2. Put hi in it"), &["Write"], ("test", &["-f", "greet.md"])))
        .exec(ScriptedExec::done("wrote").write("greet.md", ""))
        .verdict(failed_a1());
    save_one(&ws, "greet");
    run_once(&ws, "local", "", &fabric, &engine).unwrap();
    let path = record_path(&root);
    approve(&path);
    for _ in 0..4 {
        advance(&root, &path, &fabric, 10, &engine).unwrap();
    }
    assert_eq!((load(&path).state, load(&path).step), (State::Defined, Some(Step::Approve)), "{}", load(&path).log);
    approve(&path);
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    engine.push_exec(Err("Claude Code exited before answering".into()));
    assert!(advance(&root, &path, &fabric, 10, &engine).is_err());
    engine.push_exec(Ok(ScriptedExec::ask("Which greeting?").tool("Read", json!({"file_path": "greet.md"}))));
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    answer(&path, "hi");
    engine.push_exec(Ok(ScriptedExec::done("wrote hi").write("greet.md", "hi")));
    advance(&root, &path, &fabric, 10, &engine).unwrap();
    let failed: Vec<Option<String>> =
        engine.seen().into_iter().filter_map(|s| if let Seen::Execute { failed, .. } = s { Some(failed) } else { None }).collect();
    let note = Some("- a1: greet.md is empty".to_string());
    assert_eq!(failed, [None, note.clone(), note, None], "{}", load(&path).log);
}

#[test]
fn a_fresh_empty_chat_marker_counts_as_running_and_an_old_one_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let id = "12345678-1234-4234-8234-123456789abc";
    let lock = fabr_core::ccsession::chat_marker(tmp.path(), id);
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    std::fs::write(&lock, "").unwrap();
    assert!(fabr_agent::lifecycle::chat_running(tmp.path(), id) && lock.exists());
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
    assert!(!fabr_agent::lifecycle::chat_running(tmp.path(), id) && !lock.exists());
}
