use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::model::{DraftAssertion, DraftCheck, PlanDraft, ScriptedModel, ToolCall};
use fabr_agent::{approval_allows_execute, run_once};
use fabr_core::fabric::{self, FabricWriteReq};
use fabr_core::taskrec::{self, State, Step, TaskRecord};
use fabr_core::{NodeRef, Phase, Workspace};
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
        "default_folder = \"/\"\nmodel = \"claude-sonnet-5-5\"\nheartbeat_interval_s = 10\n\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\"]\n",
    )
    .unwrap();
    std::fs::write(fabric.join("secrets.toml"), "anthropic_api_key = \"test-key\"\n").unwrap();
    (tmp, fabric, ws)
}

fn draft(plan: &str, tools: &[&str], check: (&str, &[&str])) -> PlanDraft {
    PlanDraft {
        spec: "Write greet.md containing hi".into(),
        success: "greet.md contains hi".into(),
        plan: plan.into(),
        assertions: vec![DraftAssertion { id: "a1".into(), text: "greet.md contains hi".into() }],
        reversibility: "reversible".into(),
        tools: tools.iter().map(|t| (*t).to_string()).collect(),
        checks: vec![DraftCheck { cmd: check.0.into(), args: check.1.iter().map(|a| (*a).to_string()).collect() }],
    }
}

fn greet_plan() -> PlanDraft {
    draft("1. Write greet.md", &["write_file"], ("test", &["-f", "greet.md"]))
}

fn write_call() -> ToolCall {
    ToolCall { name: "write_file".into(), path: "greet.md".into(), content: Some("hi".into()), params: None }
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
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = open_reminder(&root, title).unwrap();
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

#[test]
fn fixture_gather_persists_through_the_record_writer() {
    let (_tmp, _fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/01-reminder/project/.fabric-space/tasks/2026-10-24-check-printer-firmware.md");
    let text = std::fs::read_to_string(&fixture).unwrap();
    let rec = taskrec::parse(&text).unwrap();
    let path = root.join(".fabric-space").join("tasks").join(format!("{}.md", rec.id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut rec = rec;
    rec.requests.delegate = true;
    taskrec::write_record(&path, &rec, None).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    advance(&root, &path, &root, 10, &model).unwrap();
    advance(&root, &path, &root, 10, &model).unwrap();
    let again = load(&path);
    assert_eq!(again.state, State::Task);
    assert_eq!(again.step, Some(Step::Define));
    assert!(again.context.contains("Release notes are on the maker site."));
    assert!(again.context.contains("### Gathered"));
    let round = taskrec::parse(&taskrec::render(&again)).unwrap();
    assert_eq!(round.id, again.id);
    assert_eq!(round.context, again.context);
}

#[test]
fn registry_names_differ_for_the_same_slug() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let a = left.path().join(".fabric-space").join("tasks").join("2026-10-24-alpha.md");
    let b = right.path().join(".fabric-space").join("tasks").join("2026-10-24-alpha.md");
    let name_a = taskrec::registry_filename("2026-10-24-alpha", &a.to_string_lossy());
    let name_b = taskrec::registry_filename("2026-10-24-alpha", &b.to_string_lossy());
    assert_ne!(name_a, name_b);
}

#[test]
fn planner_stops_at_pending_approval() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!(rec.state, State::Defined);
    assert_eq!(rec.step, Some(Step::Approve));
    assert!(rec.waiting);
    assert!(!rec.spec.trim().is_empty());
    assert!(!rec.success.trim().is_empty());
    assert!(!rec.plan.trim().is_empty());
    assert!(!rec.assertions.is_empty());
    assert!(rec.plan_meta.as_ref().unwrap().checks.len() == 1);
    assert_eq!(rec.plan_meta.as_ref().unwrap().reversibility.as_deref(), Some("reversible"));
    let digest = rec.approval.as_ref().unwrap().digest.clone().unwrap();
    assert_eq!(digest, taskrec::approval_digest(&rec));
    assert_eq!(rec.approval.as_ref().unwrap().status, "pending");
    assert_eq!(model.execute_count(), 0);
    assert!(!root.join("greet.md").exists());
}

#[test]
fn define_question_pauses_without_a_new_state() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::with_question(greet_plan(), "Which filename?");
    save_one(&ws, "ask first");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!(rec.state, State::Task);
    assert_eq!(rec.step, Some(Step::Define));
    assert!(rec.waiting);
    assert_eq!(rec.question.as_ref().unwrap().text, "Which filename?");
    assert!(rec.question.as_ref().unwrap().answer.is_none());
    assert!(rec.spec.trim().is_empty());
    assert!(rec.plan.trim().is_empty());
    assert_eq!(model.execute_count(), 0);
}

#[test]
fn a_plan_still_invalid_after_one_repair_round_parks_with_needs_and_no_error_count() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(draft("1. Open the shop", &["browser"], ("test", &["-f", "greet.md"])), vec![write_call()]);
    model.push_plan(draft("1. Open the shop", &["browser"], ("git", &["status"])));
    save_one(&ws, "browse");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rejections = model.plan_rejections();
    assert_eq!(rejections.len(), 2);
    assert!(rejections[0].is_none());
    let sent = rejections[1].as_deref().unwrap();
    assert!(sent.contains("unsupported tool browser") && sent.contains("allowed checks: test"), "{sent}");
    let rec = load(&record_path(&root));
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!(rec.needs.as_deref(), Some("harness: browser, checks: git"));
    let meta = rec.plan_meta.as_ref().unwrap();
    assert_eq!((meta.requires.tools.clone(), meta.requires.checks.clone()), (vec!["browser".to_string()], vec!["git".to_string()]));
    assert!(rec.log.contains("plan: needs harness: browser, checks: git") && !rec.log.contains("error"), "{}", rec.log);
    assert_eq!(rec.by.plan, Some(taskrec::Author::Agent));
    assert_eq!(model.execute_count(), 0);
}

#[test]
fn a_repair_round_fixes_a_disallowed_check() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(draft("1. write", &["write_file"], ("git", &["status"])), vec![write_call()]);
    model.push_plan(greet_plan());
    save_one(&ws, "repair");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&record_path(&root));
    assert!(model.plan_rejections()[1].as_deref().unwrap().contains("check command git is not allowed; allowed tools"));
    assert!(rec.needs.is_none() && rec.log.contains("plan: retry after rejection: check command git is not allowed"), "{}", rec.log);
    assert_eq!(rec.plan_meta.unwrap().requires.checks, vec!["test".to_string()]);
}

#[test]
fn a_shell_or_interpreter_check_never_reaches_an_approvable_plan() {
    for (cmd, args) in [("sh", vec!["-c", "id"]), ("env", vec!["sh"]), ("python3", vec!["-c", "1"]), ("cat", vec!["/etc/passwd"])] {
        let (_tmp, fabric, ws) = setup();
        let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
        let bad = || draft("1. check", &["write_file"], (cmd, &args));
        let model = ScriptedModel::new(bad(), vec![write_call()]);
        model.push_plan(bad());
        save_one(&ws, "check");
        run_once(&ws, "local", "", &fabric, &model).unwrap();
        let rec = load(&record_path(&root));
        assert!(rec.needs.is_some(), "{cmd}");
        edit(&record_path(&root), |r| r.approval.as_mut().unwrap().status = "approved".into());
        let _ = run_once(&ws, "local", "", &fabric, &model);
        assert_eq!(load(&record_path(&root)).state, State::Defined, "{cmd}");
        assert_eq!(model.execute_count(), 0, "{cmd}");
    }
}

#[test]
fn approval_refuses_pending_refused_and_stale_digest() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "gate");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let path = record_path(&root);
    assert_eq!(load(&path).approval.as_ref().unwrap().status, "pending");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert_eq!(load(&path).state, State::Defined);
    assert_eq!(model.execute_count(), 0);

    let mut rec = load(&path);
    rec.approval.as_mut().unwrap().status = "refused".into();
    rec.approval.as_mut().unwrap().note = Some("shorter plan".into());
    let expected = rec.updated_at.clone();
    taskrec::write_record(&path, &rec, Some(&expected)).unwrap();
    model.push_plan(draft("1. Write greet.md differently", &["write_file"], ("test", &["-f", "greet.md"])));
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let redrafted = load(&path);
    assert_eq!(redrafted.state, State::Defined);
    assert_eq!(redrafted.approval.as_ref().unwrap().status, "pending");
    assert!(redrafted.plan.contains("differently"));
    assert_eq!(redrafted.approval.as_ref().unwrap().digest.as_deref(), Some(taskrec::approval_digest(&redrafted).as_str()));
    assert!(redrafted.log.contains("refused: shorter plan"));

    approve(&path);
    let mut stale = load(&path);
    stale.plan.push_str("\nextra");
    let expected = stale.updated_at.clone();
    taskrec::write_record(&path, &stale, Some(&expected)).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let reset = load(&path);
    assert_eq!(reset.state, State::Defined);
    assert_eq!(reset.approval.as_ref().unwrap().status, "pending");
    assert_eq!(reset.approval.as_ref().unwrap().digest.as_deref(), Some(taskrec::approval_digest(&reset).as_str()));
    assert!(reset.waiting);
    assert!(reset.log.contains("approval digest mismatch, reset to pending"), "{}", reset.log);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(load(&path).log.matches("approval digest mismatch").count(), 1);
    assert_eq!(model.execute_count(), 0);
}

#[test]
fn executor_does_not_set_the_verdict() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "split");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let path = record_path(&root);
    approve(&path);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let started = load(&path);
    assert_eq!(started.state, State::InProgress);
    assert_eq!(started.step, Some(Step::Execute));
    assert!(started.verdict.is_none());
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let executed = load(&path);
    assert_eq!(executed.state, State::InProgress);
    assert_eq!(executed.step, Some(Step::Evaluate));
    assert!(executed.verdict.is_none());
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let judged = load(&path);
    assert_eq!(judged.verdict.as_ref().unwrap().result, "pass");
    assert_eq!(judged.state, State::Succeeded);
    assert_eq!(judged.verdict.as_ref().unwrap().items[0].id, "a1");
}

#[test]
fn failed_check_does_not_succeed() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut plan = greet_plan();
    plan.assertions = vec![DraftAssertion { id: "a1".into(), text: "the file is present".into() }];
    plan.checks = vec![DraftCheck { cmd: "test".into(), args: vec!["-f".into(), "missing-file".into()] }];
    let model = ScriptedModel::new(plan, vec![write_call()]);
    save_one(&ws, "fail check");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let path = record_path(&root);
    approve(&path);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Failed);
    assert_eq!(rec.verdict.as_ref().unwrap().result, "fail");
    let node = ws.get_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    assert_eq!(node.items[0].phase, Phase::Failed);
}

#[test]
fn restart_does_not_write_the_file_twice() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(
        greet_plan(),
        vec![ToolCall { name: "write_file".into(), path: "greet.md".into(), content: Some("HI-AGAIN".into()), params: None }],
    );
    let path = save_one(&ws, "resume");
    let id = load(&path).id.clone();
    std::fs::write(root.join("greet.md"), "hi").unwrap();
    let mut rec = load(&path);
    rec.state = State::InProgress;
    rec.step = Some(Step::Execute);
    rec.spec = "Write greet.md containing hi".into();
    rec.plan = "1. Write greet.md".into();
    rec.log = "- 2026-10-24T00:00:00Z execute: write_file greet.md\n".into();
    rec.plan_meta = Some(taskrec::PlanMeta {
        reversibility: Some("reversible".into()),
        tools: vec!["write_file".into()],
        assumptions: Vec::new(),
        checks: vec![taskrec::Check { cmd: "test".into(), args: vec!["-f".into(), "greet.md".into()] }],
        requires: Default::default(),
        bound_actions: Vec::new(),
    });
    let expected = rec.updated_at.clone();
    taskrec::write_record(&path, &rec, Some(&expected)).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    let after = load(&path);
    assert_eq!(after.id, id);
    assert_eq!(after.log.matches("write_file greet.md").count(), 1);
    assert_eq!(after.step, Some(Step::Evaluate));
    assert!(after.verdict.is_none());
    let tasks = root.join(".fabric-space").join("tasks");
    let files: Vec<_> = std::fs::read_dir(&tasks).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "md")).collect();
    assert_eq!(files.len(), 1);
}

#[test]
fn end_to_end_greet_file() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let path = record_path(&root);
    let drafted = load(&path);
    assert!(drafted.source.is_none());
    assert!(!drafted.id.is_empty());
    approve(&path);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    assert_eq!(rec.state, State::Succeeded);
    assert_eq!(rec.verdict.as_ref().unwrap().result, "pass");
    assert!(rec.step.is_none());
    assert!(rec.log.contains("learn: deferred"));
    assert!(rec.log.contains("notify: result succeeded"));
    let registry = fabric.join("registry");
    let registry_files = std::fs::read_dir(&registry).map(|rd| rd.flatten().count()).unwrap_or(0);
    assert_eq!(registry_files, 0);
    let node = ws.get_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    assert_eq!(node.items[0].phase, Phase::Done);
    let id = node.items[0].id.clone();
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let final_node = ws.get_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    assert_eq!(final_node.items[0].id, id);
    assert_eq!(final_node.items[0].phase, Phase::Done);
    assert_eq!(load(&path).state, State::Succeeded);
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
        assert_eq!((original.schema, original.pri), (6, 0), "{}", fixture.display());
        assert!(original.source.is_none(), "{}", fixture.display());
        assert_eq!(fabric::updated_at(&text), original.updated_at, "{}", fixture.display());

        let (_tmp, fabric_space, ws) = setup();
        let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
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
        let model = ScriptedModel::new(greet_plan(), Vec::new());
        if !waiting_on_question(&before) && !waiting_on_approval(&before) {
            advance(&root, &dest, &fabric_space, 10, &model).unwrap();
        }
        let staged = load(&dest);
        let expected = staged.updated_at.clone();
        taskrec::write_record(&dest, &staged, Some(&expected)).unwrap();
        let disk = std::fs::read_to_string(&dest).unwrap();
        let again = taskrec::parse(&disk).unwrap();
        assert_eq!(fabric::updated_at(&disk), again.updated_at, "{}", fixture.display());
        assert_eq!(again.state, staged.state, "{}", fixture.display());
        assert_eq!(again.step, staged.step, "{}", fixture.display());

        if waiting_on_question(&before) {
            assert!(again.waiting, "{}", fixture.display());
            assert_eq!(again.state, before.state, "{}", fixture.display());
            assert_eq!(again.step, before.step, "{}", fixture.display());
            assert!(again.question.as_ref().unwrap().answer.is_none(), "{}", fixture.display());
            assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
        } else if waiting_on_approval(&before) {
            assert_eq!(again.state, State::Defined, "{}", fixture.display());
            assert_eq!(again.step, Some(Step::Approve), "{}", fixture.display());
            assert!(again.waiting, "{}", fixture.display());
            assert_eq!(again.approval.as_ref().unwrap().status, "pending", "{}", fixture.display());
            assert_eq!(again.updated_at, before.updated_at, "{}", fixture.display());
        } else {
            match (before.state, before.step) {
                (State::Reminder, _) => {
                    assert_eq!(again.state, State::Task, "{}", fixture.display());
                    assert_eq!(again.step, Some(Step::Gather), "{}", fixture.display());
                    assert!(!again.waiting, "{}", fixture.display());
                }
                (State::Task, Some(Step::Define)) => {
                    assert_eq!(again.state, State::Task, "{}", fixture.display());
                    assert_eq!(again.step, Some(Step::Plan), "{}", fixture.display());
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
                    assert_eq!(again.state, before.state, "{}", fixture.display());
                    assert_eq!(again.step, before.step, "{}", fixture.display());
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
    let settings = std::fs::read_to_string(fabric_space.join("settings.toml")).unwrap();
    assert!(settings.contains("\"folder\"") && settings.contains("\"api_key\"") && settings.contains("\"model\""));
    assert!(!settings.contains("telegram"));
    let secrets = std::fs::read_to_string(fabric_space.join("secrets.toml")).unwrap();
    assert!(!secrets.contains("telegram"));
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "add greet.md that contains hi");
    run_once(&ws, "local", "", &fabric_space, &model).unwrap();
    let path = record_path(&root);
    let drafted = load(&path);
    assert_eq!(drafted.state, State::Defined);
    assert_eq!(drafted.step, Some(Step::Approve));
    assert!(drafted.waiting);
    assert!(!drafted.spec.trim().is_empty());
    assert!(!drafted.plan.trim().is_empty());
    assert!(!drafted.assertions.is_empty());
    assert!(drafted.source.is_none());
    assert_eq!(model.execute_count(), 0);
    assert!(!root.join("greet.md").exists());

    let mut moved = drafted.clone();
    moved.state = State::InProgress;
    let refused = fabric::write(&FabricWriteReq {
        path: path.to_string_lossy().into_owned(),
        text: taskrec::render(&moved),
        expected_updated_at: drafted.updated_at.clone(),
    })
    .unwrap_err();
    assert!(refused.message().contains("state"));
    assert_eq!(load(&path).state, State::Defined);

    let mut approved = drafted.clone();
    let approval = approved.approval.as_mut().unwrap();
    approval.status = "approved".into();
    approval.by = Some("gui".into());
    approval.at = Some("2026-10-24T19:00:00Z".into());
    approval.digest = None;
    approved.source = None;
    approved.updated_at = "2026-10-24T19:00:00Z".into();
    gui_write(&path, &approved, &drafted.updated_at);
    let on_disk = load(&path);
    assert_eq!(on_disk.state, State::Defined);
    assert_eq!(on_disk.step, Some(Step::Approve));
    assert_eq!(on_disk.source, drafted.source);
    assert_eq!(on_disk.approval.as_ref().unwrap().status, "approved");
    assert_eq!(on_disk.approval.as_ref().unwrap().digest, drafted.approval.as_ref().unwrap().digest);
    assert_eq!(fabric::updated_at(&std::fs::read_to_string(&path).unwrap()), on_disk.updated_at);
    approval_allows_execute(&on_disk).unwrap();
    assert_eq!(model.execute_count(), 0);
    run_once(&ws, "local", "", &fabric_space, &model).unwrap();
    assert!(model.execute_count() >= 1);
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    assert_eq!(load(&path).state, State::Succeeded);
}

#[test]
fn gui_refusal_redrafts_without_executing() {
    let (_tmp, fabric_space, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "refuse the plan");
    run_once(&ws, "local", "", &fabric_space, &model).unwrap();
    let path = record_path(&root);
    let drafted = load(&path);
    let mut refused = drafted.clone();
    let approval = refused.approval.as_mut().unwrap();
    approval.status = "refused".into();
    approval.by = Some("gui".into());
    approval.at = Some("2026-10-24T19:00:00Z".into());
    approval.note = Some("shorter plan".into());
    refused.updated_at = "2026-10-24T19:00:00Z".into();
    gui_write(&path, &refused, &drafted.updated_at);
    assert_eq!(load(&path).state, State::Defined);
    model.push_plan(draft("1. Write greet.md differently", &["write_file"], ("test", &["-f", "greet.md"])));
    run_once(&ws, "local", "", &fabric_space, &model).unwrap();
    let redrafted = load(&path);
    assert_eq!(redrafted.state, State::Defined);
    assert_eq!(redrafted.approval.as_ref().unwrap().status, "pending");
    assert!(redrafted.plan.contains("differently"));
    assert_eq!(model.execute_count(), 0);
    assert!(!root.join("greet.md").exists());
}

struct Broken;

impl fabr_agent::Model for Broken {
    fn define(&self, _: &str, _: &str) -> Result<fabr_agent::DefineDraft, fabr_core::FabrError> {
        Err(fabr_core::FabrError::Remote("model said no, key sk-ant-api03-FAKEFAKEFAKEFAKEFAKE-x9Qa".into()))
    }

    fn plan(&self, _: &fabr_agent::PlanInput) -> Result<fabr_agent::PlanDraft, fabr_core::FabrError> {
        Err(fabr_core::FabrError::Remote("no plan".into()))
    }

    fn execute_turn(
        &self,
        _: &fabr_agent::ExecuteInput,
        _: &mut Vec<fabr_agent::Message>,
    ) -> Result<fabr_agent::ExecuteTurn, fabr_core::FabrError> {
        Err(fabr_core::FabrError::Remote("no execute".into()))
    }

    fn evaluate(&self, _: &fabr_agent::EvaluateInput) -> Result<fabr_agent::EvalVerdict, fabr_core::FabrError> {
        Err(fabr_core::FabrError::Remote("no evaluate".into()))
    }
}

fn delegated(ws: &Workspace, root: &Path) -> PathBuf {
    let path = open_reminder(root, "job").unwrap();
    let _ = ws;
    edit(&path, |r| r.requests.delegate = true);
    path
}

#[test]
fn a_reminder_without_a_delegate_request_is_not_touched() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = open_reminder(&root, "idle").unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn three_errors_on_one_step_fail_the_task_with_the_reason_in_the_log() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = delegated(&ws, &root);
    advance(&root, &path, &fabric, 10, &Broken).unwrap();
    advance(&root, &path, &fabric, 10, &Broken).unwrap();
    for n in 1..=3 {
        let err = advance(&root, &path, &fabric, 10, &Broken);
        let rec = load(&path);
        if n < 3 {
            assert!(err.is_err());
            assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Define)));
            assert!(rec.log.contains(&format!("define: error {n}:")), "{}", rec.log);
        } else {
            assert!(err.is_ok());
            assert_eq!((rec.state, rec.step), (State::Failed, None));
            assert!(rec.verdict.is_none());
            assert!(!rec.waiting);
            assert!(rec.log.contains("define: failed after 3 errors: model said no"), "{}", rec.log);
        }
        assert!(!rec.log.contains("FAKEFAKE"), "{}", rec.log);
    }
    let again = load(&path);
    advance(&root, &path, &fabric, 10, &Broken).unwrap();
    assert_eq!(load(&path).log, again.log);
}

#[test]
fn a_gui_write_between_read_and_write_is_folded_in() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = delegated(&ws, &root);
    let model = ScriptedModel::with_question(greet_plan(), "Which filename?");
    for _ in 0..3 {
        advance(&root, &path, &fabric, 10, &model).unwrap();
    }
    let mut mine = load(&path);
    let mut base = mine.clone();
    assert!(mine.question.is_some());

    let answer = taskrec::Answer {
        value: Some("greet.md".into()),
        replacement: None,
        granted: None,
        by: Some("gui".into()),
        at: Some("2026-10-24T18:05:02Z".into()),
    };
    edit(&path, |r| {
        r.requests.cancel = true;
        r.question.as_mut().unwrap().answer = Some(answer.clone());
        r.grants.granted = vec![taskrec::Grant { field: "address.zip".into(), scope: "task".into() }];
        r.approval =
            Some(taskrec::Approval { status: "approved".into(), by: Some("gui".into()), at: None, note: None, digest: Some("x".into()) });
    });

    fabr_agent::state::append_log(&mut mine.log, "define", "stale stage wrote this");
    mine.spec = "new spec".into();
    fabr_agent::store::update(&path, &mut mine, &mut base).unwrap();

    let disk = load(&path);
    assert!(disk.requests.cancel);
    assert_eq!(disk.question.as_ref().unwrap().answer.as_ref(), Some(&answer));
    assert_eq!(disk.grants.granted.len(), 1);
    assert_eq!(disk.approval.as_ref().unwrap().status, "approved");
    assert_eq!(disk.spec, "new spec");
    assert!(disk.log.contains("stale stage wrote this"));
    assert_eq!(mine, disk);
}

struct CancelsMidRun {
    path: PathBuf,
}

impl fabr_agent::Model for CancelsMidRun {
    fn define(&self, _: &str, _: &str) -> Result<fabr_agent::DefineDraft, fabr_core::FabrError> {
        unreachable!()
    }

    fn plan(&self, _: &fabr_agent::PlanInput) -> Result<fabr_agent::PlanDraft, fabr_core::FabrError> {
        unreachable!()
    }

    fn execute_turn(
        &self,
        _: &fabr_agent::ExecuteInput,
        _: &mut Vec<fabr_agent::Message>,
    ) -> Result<fabr_agent::ExecuteTurn, fabr_core::FabrError> {
        edit(&self.path, |r| r.requests.cancel = true);
        let call = |name: &str| fabr_agent::ToolUse {
            id: name.into(),
            call: ToolCall { name: "write_file".into(), path: name.into(), content: Some("x".into()), params: None },
        };
        Ok(fabr_agent::ExecuteTurn {
            outcome: fabr_agent::TurnOutcome::Tools(vec![call("first.md"), call("second.md")]),
            stop_reason: "tool_use".into(),
        })
    }

    fn evaluate(&self, _: &fabr_agent::EvaluateInput) -> Result<fabr_agent::EvalVerdict, fabr_core::FabrError> {
        unreachable!()
    }
}

#[test]
fn a_cancel_that_lands_during_execute_survives_the_per_call_persist() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = delegated(&ws, &root);
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.plan = "1. write".into();
    });
    let model = CancelsMidRun { path: path.clone() };
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let rec = load(&path);
    assert!(rec.requests.cancel, "cancel was erased");
    assert_eq!(rec.state, State::InProgress);
    assert!(root.join("first.md").exists());
    assert!(!root.join("second.md").exists());
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Failed);
    assert!(!rec.requests.cancel);
    assert!(rec.log.contains("cancelled by user"));
    assert!(rec.verdict.is_none());
}

#[test]
fn a_hung_check_is_killed_and_counted_as_a_stage_error() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    std::fs::write(root.join("greet.md"), "hi").unwrap();
    let path = delegated(&ws, &root);
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Evaluate);
        r.plan_meta = Some(taskrec::PlanMeta {
            reversibility: Some("reversible".into()),
            tools: vec![],
            assumptions: vec![],
            checks: vec![taskrec::Check { cmd: "tail".into(), args: vec!["-f".into(), "greet.md".into()] }],
            requires: Default::default(),
            bound_actions: Vec::new(),
        });
    });
    let model = ScriptedModel::new(greet_plan(), vec![]);
    let mut ctl = fabr_agent::Ctl::new(&fabric, 10);
    ctl.check_timeout_s = 1;
    let started = std::time::Instant::now();
    let err = fabr_agent::advance_with(&root, &path, &ctl, &model).unwrap_err();
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(err.message().contains("timed out"), "{}", err.message());
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::InProgress, Some(Step::Evaluate)));
    assert!(rec.log.contains("evaluate: error 1: check timed out"), "{}", rec.log);
}

#[derive(Default)]
struct Recording {
    inner: Option<ScriptedModel>,
    define_contexts: std::sync::Mutex<Vec<String>>,
    plan_answers: std::sync::Mutex<Vec<String>>,
}

impl fabr_agent::Model for Recording {
    fn define(&self, title: &str, context: &str) -> Result<fabr_agent::DefineDraft, fabr_core::FabrError> {
        self.define_contexts.lock().unwrap().push(context.to_string());
        self.inner.as_ref().unwrap().define(title, context)
    }

    fn plan(&self, input: &fabr_agent::PlanInput) -> Result<fabr_agent::PlanDraft, fabr_core::FabrError> {
        self.plan_answers.lock().unwrap().push(input.answers.to_string());
        self.inner.as_ref().unwrap().plan(input)
    }

    fn execute_turn(
        &self,
        input: &fabr_agent::ExecuteInput,
        messages: &mut Vec<fabr_agent::Message>,
    ) -> Result<fabr_agent::ExecuteTurn, fabr_core::FabrError> {
        self.inner.as_ref().unwrap().execute_turn(input, messages)
    }

    fn evaluate(&self, input: &fabr_agent::EvaluateInput) -> Result<fabr_agent::EvalVerdict, fabr_core::FabrError> {
        self.inner.as_ref().unwrap().evaluate(input)
    }
}

#[test]
fn the_users_answer_reaches_define_and_plan() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let path = delegated(&ws, &root);
    let model = Recording { inner: Some(ScriptedModel::with_question(greet_plan(), "Which filename?")), ..Recording::default() };
    for _ in 0..3 {
        advance(&root, &path, &fabric, 10, &model).unwrap();
    }
    let asked = load(&path);
    assert_eq!(asked.question.as_ref().unwrap().id, "q1");
    edit(&path, |r| {
        r.question.as_mut().unwrap().answer = Some(taskrec::Answer {
            value: Some("call it hello-42.txt".into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: None,
        });
    });
    advance(&root, &path, &fabric, 10, &model).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let contexts = model.define_contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 2);
    assert!(!contexts[0].contains("User answered"));
    assert!(contexts[1].contains("Already asked: Which filename?\nUser answered: call it hello-42.txt"), "{}", contexts[1]);
    let plans = model.plan_answers.lock().unwrap().clone();
    assert_eq!(plans.len(), 1);
    assert!(plans[0].contains("User answered: call it hello-42.txt"), "{}", plans[0]);
    let done = load(&path);
    assert_eq!(done.answers.len(), 1);
    assert_eq!(done.state, State::Defined);
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
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    save_one(&ws, "track steps");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let path = record_path(&root);
    assert_eq!(registry_step(&fabric), Some(("defined".into(), "approve".into())));
    approve(&path);
    let mut seen = Vec::new();
    for _ in 0..6 {
        advance(&root, &path, &fabric, 10, &model).unwrap();
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
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]).with_grants(&["address.zip", "address.zip", "name.first"]);
    save_one(&ws, "needs data");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&record_path(&root));
    assert_eq!(rec.grants.requested, vec!["address.zip".to_string(), "name.first".to_string()]);
    assert!(rec.grants.granted.is_empty());
}

fn entry(fabric: &Path, path: &Path) -> Option<serde_yaml::Value> {
    let key = fabr_agent::task_path_key(path);
    let file = fabr_agent::registry_path(fabric, &load(path).id, &key);
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
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut plan = draft("1. look", &["list_dir"], ("test", &["-d", "."]));
    plan.assertions[0].text = "the folder exists".into();
    let model = ScriptedModel::with_question(plan, "Which filename?");
    let path = save_one(&ws, "observe me");
    let tick = |name: &str| {
        advance(&root, &path, &fabric, 10, &model).unwrap();
        name.to_string()
    };
    let t = tick("delegate");
    assert_eq!(load(&path).step, Some(Step::Gather), "{t}");
    for name in ["gather", "define-question"] {
        let t = tick(name);
        assert_mirrors(&fabric, &path, &t);
    }
    assert_eq!(entry(&fabric, &path).unwrap()["waiting"].as_str(), Some("question"));
    edit(&path, |r| {
        r.question.as_mut().unwrap().answer =
            Some(taskrec::Answer { value: Some("greet.md".into()), replacement: None, granted: None, by: Some("gui".into()), at: None });
    });
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

    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    let second = open_reminder(&root, "cancel me").unwrap();
    edit(&second, |r| r.requests.delegate = true);
    for _ in 0..4 {
        advance(&root, &second, &fabric, 10, &model).unwrap();
    }
    approve(&second);
    advance(&root, &second, &fabric, 10, &model).unwrap();
    assert_eq!(load(&second).step, Some(Step::Execute));
    assert_mirrors(&fabric, &second, "before cancel");
    edit(&second, |r| r.requests.cancel = true);
    advance(&root, &second, &fabric, 10, &model).unwrap();
    let rec = load(&second);
    assert_eq!(rec.state, State::Failed);
    assert!(rec.log.contains("execute: cancelled by user"));
    assert!(entry(&fabric, &second).is_none());
}

fn drive_n(root: &Path, path: &Path, ctl: &fabr_agent::Ctl, model: &ScriptedModel, n: usize) {
    for _ in 0..n {
        let _ = fabr_agent::advance_with(root, path, ctl, model);
    }
}

#[test]
fn a_read_only_machine_plans_and_auto_approves_then_parks_at_execute_and_a_capable_one_executes() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    let path = save_one(&ws, "greet");
    let mut ro = fabr_agent::Ctl::new(&fabric, 10);
    ro.harness.mode = fabr_core::setup::ExecuteMode::ReadOnly;
    ro.harness.auto_mode = true;
    drive_n(&root, &path, &ro, &model, 10);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!(rec.approval.as_ref().map(|a| (a.status.as_str(), a.by.as_deref())), Some(("approved", Some("auto"))));
    assert_eq!(rec.needs.as_deref(), Some("harness: write_file, checks: test"));
    assert_eq!(rec.log.matches("execute: needs harness: write_file, checks: test").count(), 1, "{}", rec.log);
    assert_eq!(Phase::of(&rec, "2026-10-25T12:00"), Phase::NeedsHarness);
    assert_eq!(model.execute_count(), 0);
    assert!(!root.join("greet.md").exists());
    let full = fabr_agent::Ctl::new(&fabric, 10);
    drive_n(&root, &path, &full, &model, 12);
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded, "{}", rec.log);
    assert!(rec.needs.is_none());
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
}

#[test]
fn human_sections_are_not_regenerated_and_redo_steps_back() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![write_call()]);
    let path = save_one(&ws, "greet");
    let ctl = fabr_agent::Ctl::new(&fabric, 10);
    drive_n(&root, &path, &ctl, &model, 2);
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
    drive_n(&root, &path, &ctl, &model, 3);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Defined, Some(Step::Approve)));
    assert_eq!((rec.spec.as_str(), rec.success.as_str(), rec.plan.as_str()), ("my own spec", "my own success", "my own plan"));
    assert_eq!(rec.assertions[0].id, "h1");
    assert_eq!(rec.by.plan, Some(taskrec::Author::Human));
    assert_eq!(rec.plan_meta.as_ref().unwrap().requires.checks, vec!["test".to_string()]);
    edit(&path, |r| r.requests.redo = Some(taskrec::RedoStage::Plan));
    drive_n(&root, &path, &ctl, &model, 1);
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::Task, Some(Step::Plan)));
    assert!(rec.plan.is_empty() && rec.approval.is_none() && rec.plan_meta.is_none() && rec.requests.redo.is_none());
    assert_eq!(rec.spec, "my own spec");
    assert!(rec.log.contains("redo: plan by user"), "{}", rec.log);
    edit(&path, |r| {
        r.state = State::InProgress;
        r.step = Some(Step::Execute);
        r.requests.redo = Some(taskrec::RedoStage::Define);
    });
    drive_n(&root, &path, &ctl, &model, 1);
    let rec = load(&path);
    assert!(rec.requests.redo.is_none() && rec.log.contains("ignored request redo in state in_progress"), "{}", rec.log);
}

fn run_to_approval(ws: &Workspace, fabric: &Path, model: &impl fabr_agent::Model, root: &Path) -> PathBuf {
    save_one(ws, "turns");
    run_once(ws, "local", "", fabric, model).unwrap();
    let path = record_path(root);
    approve(&path);
    path
}

fn tool(name: &str, path: &str, content: Option<&str>) -> ToolCall {
    ToolCall { name: name.into(), path: path.into(), content: content.map(str::to_string), params: None }
}

fn judged(id: &str, j: fabr_agent::Judgement, evidence: &str) -> fabr_agent::JudgedItem {
    fabr_agent::JudgedItem { id: id.into(), judgement: j, evidence: evidence.into() }
}

#[test]
fn execute_loops_over_turns_and_the_model_sees_the_tool_results() {
    use fabr_agent::ScriptedTurn::{Done, Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    std::fs::write(root.join("notes.txt"), "remember the milk").unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![
        Tools(vec![tool("list_dir", ".", None), tool("read_file", "notes.txt", None)]),
        Tools(vec![tool("write_file", "greet.md", Some("hi"))]),
        Done,
    ]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded);
    for line in
        ["execute: turn 1", "execute: list_dir .", "execute: read_file notes.txt", "execute: turn 3", "execute: write_file greet.md"]
    {
        assert!(rec.log.contains(line), "{line}: {}", rec.log);
    }
    let seen = model.seen().join("\n");
    assert!(seen.contains("notes.txt") && seen.contains("remember the milk"), "{seen}");
    assert_eq!(model.execute_count(), 3);
}

#[test]
fn a_refused_write_reaches_the_model_as_a_tool_error_and_the_run_continues() {
    use fabr_agent::ScriptedTurn::{Done, Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![
        Tools(vec![tool("write_file", "notes.txt", Some("x"))]),
        Tools(vec![tool("write_file", "greet.md", Some("hi"))]),
        Done,
    ]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert!(!root.join("notes.txt").exists());
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded);
    assert!(rec.log.contains("tool failed: write_file notes.txt: write_file only writes .md files; got notes.txt"), "{}", rec.log);
    assert!(model.seen().join("\n").contains("write_file only writes .md files; got notes.txt"));
}

#[test]
fn make_dir_then_write_file_lands_a_markdown_file_in_the_folder() {
    use fabr_agent::ScriptedTurn::{Done, Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![
        Tools(vec![tool("make_dir", "notes/deep", None), tool("write_file", "notes/plan.md", Some("# plan"))]),
        Tools(vec![tool("write_file", "greet.md", Some("hi"))]),
        Done,
    ]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert!(root.join("notes/deep").is_dir());
    assert_eq!(std::fs::read_to_string(root.join("notes/plan.md")).unwrap(), "# plan");
    assert_eq!(load(&path).state, State::Succeeded);
}

#[test]
fn the_turn_cap_is_a_stage_error() {
    use fabr_agent::ScriptedTurn::Tools;
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut settings = std::fs::read_to_string(fabric.join("settings.toml")).unwrap();
    settings.insert_str(0, "execute_max_turns = 2\n");
    std::fs::write(fabric.join("settings.toml"), settings).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![Tools(vec![tool("list_dir", ".", None)]); 5]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    assert!(run_once(&ws, "local", "", &fabric, &model).is_err());
    let rec = load(&path);
    assert_eq!((rec.state, rec.step), (State::InProgress, Some(Step::Execute)));
    assert!(rec.log.contains("execute: turn cap reached") && rec.log.contains("execute: error 1: turn cap reached"), "{}", rec.log);
    assert_eq!(model.execute_count(), 2);
}

struct CancelsOnTurn {
    inner: ScriptedModel,
    path: PathBuf,
}

impl fabr_agent::Model for CancelsOnTurn {
    fn define(&self, t: &str, c: &str) -> Result<fabr_agent::DefineDraft, fabr_core::FabrError> {
        self.inner.define(t, c)
    }

    fn plan(&self, i: &fabr_agent::PlanInput) -> Result<fabr_agent::PlanDraft, fabr_core::FabrError> {
        self.inner.plan(i)
    }

    fn execute_turn(
        &self,
        input: &fabr_agent::ExecuteInput,
        messages: &mut Vec<fabr_agent::Message>,
    ) -> Result<fabr_agent::ExecuteTurn, fabr_core::FabrError> {
        let turn = self.inner.execute_turn(input, messages)?;
        if self.inner.execute_count() == 1 {
            edit(&self.path, |r| r.requests.cancel = true);
        }
        Ok(turn)
    }

    fn evaluate(&self, i: &fabr_agent::EvaluateInput) -> Result<fabr_agent::EvalVerdict, fabr_core::FabrError> {
        self.inner.evaluate(i)
    }
}

#[test]
fn cancel_between_turns_fails_the_task_and_makes_no_further_call() {
    use fabr_agent::ScriptedTurn::{Done, Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let inner = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![
        Tools(vec![]),
        Tools(vec![tool("write_file", "greet.md", Some("hi"))]),
        Done,
    ]);
    let path = {
        save_one(&ws, "cancel");
        run_once(&ws, "local", "", &fabric, &inner).unwrap();
        record_path(&root)
    };
    approve(&path);
    let model = CancelsOnTurn { inner, path: path.clone() };
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Failed);
    assert!(rec.log.contains("execute: cancelled by user") && !rec.requests.cancel, "{}", rec.log);
    assert_eq!(model.inner.execute_count(), 1);
    assert!(!root.join("greet.md").exists());
}

#[test]
fn a_question_mid_run_parks_and_the_answer_continues_the_loop() {
    use fabr_agent::ScriptedTurn::{Ask, Done, Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let model = ScriptedModel::new(greet_plan(), vec![]).with_turns(vec![
        Ask("Which greeting?".into()),
        Tools(vec![tool("write_file", "greet.md", Some("hi"))]),
        Done,
    ]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let parked = load(&path);
    assert_eq!((parked.state, parked.step), (State::InProgress, Some(Step::Execute)));
    assert!(parked.waiting && parked.question.as_ref().unwrap().text == "Which greeting?");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    assert_eq!(model.execute_count(), 1, "parked run must not call the model");
    edit(&path, |r| {
        r.question.as_mut().unwrap().answer = Some(taskrec::Answer {
            value: Some("say hi please".into()),
            replacement: None,
            granted: None,
            by: Some("gui".into()),
            at: Some("2026-10-24T18:05:02Z".into()),
        })
    });
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    assert_eq!(rec.state, State::Succeeded);
    assert_eq!(rec.answers.len(), 1);
    assert!(model.seen().join("\n").contains("say hi please"));
}

#[test]
fn the_model_evaluator_decides_the_verdict_with_per_assertion_evidence() {
    use fabr_agent::{Judgement::Pass, ScriptedTurn::Tools};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let verdict = fabr_agent::EvalVerdict { items: vec![judged("a1", Pass, "greet.md holds hi")] };
    let model = ScriptedModel::new(greet_plan(), vec![])
        .with_turns(vec![Tools(vec![tool("write_file", "greet.md", Some("hi"))])])
        .with_verdicts(vec![verdict]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    assert_eq!(rec.assertions.len(), 1);
    let v = rec.verdict.as_ref().unwrap();
    assert_eq!((rec.state, v.result.as_str(), v.items.len()), (State::Succeeded, "pass", 1));
    assert_eq!(v.items[0].evidence, "greet.md holds hi");
    assert_eq!(model.evaluate_count(), 1);
    assert!(model.seen().join("\n").contains("write_file greet.md"));
}

#[test]
fn one_failing_assertion_fails_the_task() {
    use fabr_agent::Judgement::{Fail, Pass};
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut plan = greet_plan();
    plan.assertions.push(DraftAssertion { id: "a2".into(), text: "greet.md is the only file written".into() });
    let verdict = fabr_agent::EvalVerdict { items: vec![judged("a1", Pass, "ok"), judged("a2", Fail, "also wrote x.txt")] };
    let model = ScriptedModel::new(plan, vec![write_call()]).with_verdicts(vec![verdict]);
    let path = run_to_approval(&ws, &fabric, &model, &root);
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&path);
    let v = rec.verdict.as_ref().unwrap();
    assert_eq!((rec.state, v.result.as_str()), (State::Failed, "fail"));
    assert_eq!((v.items[0].pass, v.items[1].pass, v.items[1].evidence.as_str()), (true, false, "also wrote x.txt"));
}

#[test]
fn a_check_on_a_file_that_is_not_in_the_folder_or_created_is_dropped() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut plan = greet_plan();
    plan.checks.push(DraftCheck { cmd: "grep".into(), args: vec!["-q".into(), "done".into(), "tasks.md".into()] });
    let model = ScriptedModel::new(plan, vec![]);
    save_one(&ws, "ground");
    run_once(&ws, "local", "", &fabric, &model).unwrap();
    let rec = load(&record_path(&root));
    let checks = &rec.plan_meta.as_ref().unwrap().checks;
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].args, vec!["-f".to_string(), "greet.md".to_string()]);
    assert!(rec.log.contains("plan: dropped check grep -q done tasks.md: unknown file"), "{}", rec.log);
}

#[test]
fn more_than_three_assertions_is_a_stage_error() {
    let (_tmp, fabric, ws) = setup();
    let root = ws.local_path(&NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let mut plan = greet_plan();
    plan.assertions = (1..=4).map(|n| DraftAssertion { id: format!("a{n}"), text: format!("claim {n}") }).collect();
    let model = ScriptedModel::new(plan, vec![]);
    save_one(&ws, "too many");
    let err = run_once(&ws, "local", "", &fabric, &model).unwrap_err();
    assert!(err.message().contains("4 assertions; 1 to 3 are allowed"), "{}", err.message());
    assert!(load(&record_path(&root)).log.contains("define: error 1"));
}
