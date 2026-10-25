use fabr_agent::approval_allows_execute;
use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::model::{PlanDraft, ScriptedModel, ToolCall};
use fabr_agent::tools_contract::{self, GrantClass};
use fabr_core::taskrec::{self, Approval, Check, Grant, PlanMeta, State, Step, TaskRecord};
use fabr_core::{NodeRef, Workspace};
use std::path::{Path, PathBuf};

fn workspace(root: &Path) -> Workspace {
    let cfg = root.join("cfg");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    Workspace::from_config(&cfg, &cache, Some(root.to_path_buf())).unwrap()
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let ws = workspace(&root);
    ws.init_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let fabric = tmp.path().join("fabric-space");
    std::fs::create_dir_all(&fabric).unwrap();
    let path = open_reminder(&root, "bind").unwrap();
    (tmp, fabric, path)
}

fn load(path: &Path) -> TaskRecord {
    taskrec::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn save(path: &Path, rec: &TaskRecord) {
    let expected = load(path).updated_at;
    taskrec::write_record(path, rec, Some(&expected)).unwrap();
}

fn meta(tools: &[&str], checks: Vec<Check>, bound: Vec<taskrec::BoundAction>) -> PlanMeta {
    PlanMeta {
        reversibility: Some("reversible".into()),
        tools: tools.iter().map(|t| (*t).to_string()).collect(),
        assumptions: Vec::new(),
        checks,
        requires: Default::default(),
        bound_actions: bound,
    }
}

fn check_file() -> Check {
    Check { cmd: "test".into(), args: vec!["-f".into(), "greet.md".into()] }
}

fn approve(rec: &mut TaskRecord) {
    let digest = taskrec::approval_digest(rec);
    rec.approval = Some(Approval {
        status: "approved".into(),
        by: Some("gui".into()),
        at: Some("2026-10-24T00:00:00Z".into()),
        note: None,
        digest: Some(digest),
    });
}

fn call(name: &str, params: serde_json::Value) -> ToolCall {
    ToolCall { name: name.into(), path: String::new(), content: None, params: Some(params) }
}

fn quiet_plan() -> PlanDraft {
    PlanDraft {
        spec: "Write greet.md containing hi".into(),
        success: "greet.md contains hi".into(),
        plan: "1. Write greet.md".into(),
        assertions: Vec::new(),
        reversibility: "reversible".into(),
        tools: vec!["write_file".into()],
        checks: vec![fabr_agent::model::DraftCheck { cmd: "test".into(), args: vec!["-f".into(), "greet.md".into()] }],
    }
}

#[test]
fn changed_parameters_invalidate_the_approval() {
    let (_tmp, _fabric, path) = setup();
    let params = serde_json::json!({"path": "greet.md", "content": "hi"});
    let mut plan = meta(&["write_file"], vec![check_file()], Vec::new());
    tools_contract::bind_action(&mut plan, "write_file", &params).unwrap();
    let mut rec = load(&path);
    rec.plan_meta = Some(plan);
    rec.spec = "Write greet.md containing hi".into();
    rec.success = "greet.md contains hi".into();
    rec.plan = "1. Write greet.md".into();
    approve(&mut rec);
    approval_allows_execute(&rec).unwrap();
    let again = taskrec::parse(&taskrec::render(&rec)).unwrap();
    assert_eq!(taskrec::approval_digest(&rec), taskrec::approval_digest(&again));
    rec.plan_meta.as_mut().unwrap().bound_actions[0].params = "{\"content\":\"no\",\"path\":\"greet.md\"}".into();
    let err = approval_allows_execute(&rec).unwrap_err();
    assert!(err.message().contains("digest"), "{err}");
}

#[test]
fn legacy_approval_does_not_run_a_sponsor_action() {
    let (_tmp, fabric, path) = setup();
    let root = path.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    std::fs::write(fabric.join("secrets.toml"), "brightdata_api_token = \"test-token-value\"\n").unwrap();
    let mut rec = load(&path);
    rec.state = State::InProgress;
    rec.step = Some(Step::Execute);
    rec.spec = "Find public docs".into();
    rec.plan = "1. Search".into();
    rec.plan_meta = Some(meta(&["bright_data_search"], vec![check_file()], Vec::new()));
    rec.grants.granted.push(Grant { field: "external_action".into(), scope: "task".into() });
    approve(&mut rec);
    approval_allows_execute(&rec).unwrap();
    save(&path, &rec);
    let model = ScriptedModel::new(quiet_plan(), vec![call("bright_data_search", serde_json::json!({"query": "Rust ureq documentation"}))]);
    let err = advance(&root, &path, &fabric, 10, &model).unwrap_err();
    assert!(err.message().contains("does not cover"), "{err}");
    assert!(!root.join("research.md").exists());
    assert!(!load(&path).log.contains("test-token-value"));
}

#[test]
fn approved_extract_url_does_not_allow_another() {
    let (_tmp, fabric, path) = setup();
    let root = path.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    std::fs::write(fabric.join("secrets.toml"), "brightdata_api_token = \"test-token-value\"\n").unwrap();
    let approved_url = serde_json::json!({"url": "https://docs.rs/ureq"});
    let mut plan = meta(&["bright_data_extract"], vec![check_file()], Vec::new());
    tools_contract::bind_action(&mut plan, "bright_data_extract", &approved_url).unwrap();
    let mut rec = load(&path);
    rec.state = State::InProgress;
    rec.step = Some(Step::Execute);
    rec.plan_meta = Some(plan);
    rec.grants.granted.push(Grant { field: "external_action".into(), scope: "task".into() });
    approve(&mut rec);
    approval_allows_execute(&rec).unwrap();
    save(&path, &rec);
    let model =
        ScriptedModel::new(quiet_plan(), vec![call("bright_data_extract", serde_json::json!({"url": "https://example.com/other"}))]);
    let err = advance(&root, &path, &fabric, 10, &model).unwrap_err();
    assert!(err.message().contains("does not cover"), "{err}");
    assert!(!root.join("research.md").exists());
    assert!(!load(&path).log.contains("test-token-value"));
}

#[test]
fn timed_out_external_effect_does_not_pass() {
    let (_tmp, fabric, path) = setup();
    let root = path.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    std::fs::write(root.join("greet.md"), "hi").unwrap();
    let mut rec = load(&path);
    rec.state = State::InProgress;
    rec.step = Some(Step::Execute);
    rec.log = "- 2026-10-24T00:00:00Z execute: external inv_abc fo_action intent\n".into();
    rec.plan_meta = Some(meta(&["write_file"], vec![check_file()], Vec::new()));
    approve(&mut rec);
    save(&path, &rec);
    let model = ScriptedModel::new(
        quiet_plan(),
        vec![ToolCall { name: "write_file".into(), path: "greet.md".into(), content: Some("no".into()), params: None }],
    );
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(model.execute_count(), 0);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let after = load(&path);
    assert_ne!(after.state, State::Succeeded);
    assert_ne!(after.verdict.as_ref().unwrap().result, "pass");
    assert!(after.log.contains("UNKNOWN"), "{}", after.log);
    assert!(after.log.contains("reconciliation"), "{}", after.log);
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
}

#[test]
fn local_read_does_not_authorize_remote_read() {
    let keys = tools_contract::SponsorKeys::parse("brightdata_api_token = \"present\"\n");
    let tool = tools_contract::catalog(&keys).into_iter().find(|tool| tool.name == "bright_data_search").unwrap();
    let approved = vec!["bright_data_search".to_string()];
    assert!(!tools_contract::authorized(&tool, &[Grant { field: "local_read".into(), scope: "task".into() }], &approved));
    assert!(!tools_contract::grant_authorizes(GrantClass::LocalRead, GrantClass::ExternalAction));
    let callable = tools_contract::model_callable(&keys, &[Grant { field: "local_read".into(), scope: "task".into() }], &approved);
    assert!(callable.iter().all(|tool| tool.name != "bright_data_search"));
}

#[test]
fn saved_budget_changes_a_later_task() {
    let (_tmp, fabric, path_a) = setup();
    let root = path_a.parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let budget = "Stay under $100 for internship supplies";
    let id = fabr_agent::connectors::memory::memory_id(budget);
    let mut rec = load(&path_a);
    rec.state = State::InProgress;
    rec.step = Some(Step::Execute);
    rec.spec = "Remember the internship supply cap".into();
    rec.plan = "1. Propose and confirm the cap".into();
    rec.plan_meta = Some(meta(&["propose_memory", "confirm_memory"], vec![check_file()], Vec::new()));
    approve(&mut rec);
    save(&path_a, &rec);
    let model_a = ScriptedModel::new(
        quiet_plan(),
        vec![
            call("propose_memory", serde_json::json!({"text": budget, "source": "explicit"})),
            call("confirm_memory", serde_json::json!({"id": id})),
        ],
    );
    advance(&root, &path_a, &fabric, 10, &model_a).unwrap();
    let task_a = load(&path_a);
    assert!(task_a.log.contains("propose_memory"), "{}", task_a.log);
    assert!(task_a.log.contains("confirm_memory"), "{}", task_a.log);

    let path_b = open_reminder(&root, "task b desk").unwrap();
    let mut rec_b = load(&path_b);
    rec_b.state = State::InProgress;
    rec_b.step = Some(Step::Execute);
    rec_b.spec = "Find a desk setup".into();
    rec_b.plan = "1. Recall the supply preference".into();
    rec_b.plan_meta = Some(meta(&["recall_preferences"], Vec::new(), Vec::new()));
    approve(&mut rec_b);
    save(&path_b, &rec_b);
    let model_b = ScriptedModel::new(quiet_plan(), vec![call("recall_preferences", serde_json::json!({"query": "internship"}))]);
    advance(&root, &path_b, &fabric, 10, &model_b).unwrap();
    let task_b = load(&path_b);
    assert!(task_b.log.contains("source=local"), "{}", task_b.log);
    assert!(task_b.log.contains("$100"), "{}", task_b.log);
    assert!(!task_b.spec.contains("$100"));
}
