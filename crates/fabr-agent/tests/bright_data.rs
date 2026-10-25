//! Bright Data stays optional. A scripted task plans a public search, waits
//! for approval, then writes `research.md` from the stand-in MCP payload.

use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::model::{BoundDraft, DraftAssertion, DraftCheck, PlanDraft, ScriptedModel, ToolCall};
use fabr_agent::tools_contract;
use fabr_core::taskrec::{self, Grant, State, Step, TaskRecord};
use fabr_core::{NodeRef, Workspace};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const QUERY: &str = "Rust ureq documentation and GitHub issues";

fn workspace(root: &Path) -> Workspace {
    let cfg = root.join("cfg");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    Workspace::from_config(&cfg, &cache, Some(root.to_path_buf())).unwrap()
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let ws = workspace(&root);
    ws.init_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let fabric = tmp.path().join("fabric-space");
    std::fs::create_dir_all(&fabric).unwrap();
    let path = open_reminder(&root, "research ureq").unwrap();
    (tmp, fabric, root, path)
}

struct Mcp {
    stop: Arc<AtomicBool>,
    url: String,
}

impl Mcp {
    fn start() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", server.server_addr().to_ip().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let Ok(Some(mut req)) = server.recv_timeout(std::time::Duration::from_millis(50)) else { continue };
                let mut body = String::new();
                std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
                let text = if body.contains("initialize") {
                    r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}"#.to_string()
                } else {
                    r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"[ureq docs](https://docs.rs/ureq)\n[upstream issue](https://github.com/algesten/ureq/issues/1)"}]}}"#
                        .to_string()
                };
                let resp = tiny_http::Response::from_string(text)
                    .with_status_code(200u16)
                    .with_header(tiny_http::Header::from_bytes("content-type", "application/json").unwrap());
                let _ = req.respond(resp);
            }
        });
        Self { stop, url }
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe { std::env::remove_var("FABR_BRIGHTDATA_MCP_URL") }
    }
}

fn load(path: &Path) -> TaskRecord {
    taskrec::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn save(path: &Path, rec: &TaskRecord) {
    let expected = load(path).updated_at;
    taskrec::write_record(path, rec, Some(&expected)).unwrap();
}

#[test]
fn search_waits_for_approval_then_records_public_sources() {
    let mcp = Mcp::start();
    unsafe { std::env::set_var("FABR_BRIGHTDATA_MCP_URL", &mcp.url) }
    let (_tmp, fabric, root, path) = setup();
    std::fs::write(fabric.join("secrets.toml"), "brightdata_api_token = \"test-token-value\"\n").unwrap();
    let view = tools_contract::planner_view(&tools_contract::load_sponsor_keys(&fabric));
    assert!(view.contains("bright_data_search (enabled)"));
    assert!(!view.contains("test-token-value"));
    assert!(!view.contains("secrets.toml"));
    let mut rec = load(&path);
    rec.state = State::Task;
    rec.step = Some(Step::Plan);
    rec.spec = "Find public ureq documentation and an upstream issue.".into();
    rec.success = "research.md names the documentation and the issue.".into();
    save(&path, &rec);

    let plan = PlanDraft {
        spec: rec.spec.clone(),
        success: rec.success.clone(),
        plan: "1. Search public docs and issues. 2. Write research.md.".into(),
        assertions: vec![DraftAssertion { id: "a1".into(), text: "research.md contains https://docs.rs/ureq".into() }],
        reversibility: "reversible".into(),
        tools: vec!["bright_data_search".into()],
        checks: vec![DraftCheck { cmd: "test".into(), args: vec!["-f".into(), "research.md".into()] }],
    };
    let call = ToolCall {
        name: "bright_data_search".into(),
        path: String::new(),
        content: None,
        params: Some(serde_json::json!({"query": QUERY})),
    };
    let model = ScriptedModel::new(plan, vec![call])
        .with_bounds(vec![BoundDraft { tool: "bright_data_search".into(), params: serde_json::json!({"query": QUERY}) }]);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let planned = load(&path);
    assert_eq!(planned.approval.as_ref().unwrap().status, "pending");
    assert!(!planned.plan_meta.as_ref().unwrap().bound_actions.is_empty());
    assert_eq!(planned.plan_meta.as_ref().unwrap().bound_actions[0].destination, QUERY);
    assert!(!root.join("research.md").exists(), "search must not run before approval");
    assert!(!planned.log.contains("test-token-value"));

    let mut approved = planned;
    approved.approval.as_mut().unwrap().status = "approved".into();
    approved.approval.as_mut().unwrap().by = Some("gui".into());
    approved.grants.granted.push(Grant { field: "external_action".into(), scope: "task".into() });
    save(&path, &approved);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();

    let note = std::fs::read_to_string(root.join("research.md")).unwrap();
    assert!(note.contains("source: bright_data"));
    assert!(note.contains("retrieved_at:"));
    assert!(note.contains("https://docs.rs/ureq"));
    assert!(note.contains("https://github.com/algesten/ureq/issues/1"));
    assert!(!note.contains("test-token-value"));
    let done = load(&path);
    assert!(done.log.contains("bright_data_search"), "{}", done.log);
    assert!(!done.log.contains("test-token-value"), "{}", done.log);
    assert_eq!(done.grants.granted.len(), 1);
}

#[test]
fn local_write_still_runs_without_bright_data() {
    let (_tmp, fabric, root, path) = setup();
    let mut rec = load(&path);
    rec.state = State::Task;
    rec.step = Some(Step::Plan);
    rec.spec = "Write greet.md containing hi".into();
    rec.success = "greet.md contains hi".into();
    save(&path, &rec);
    let plan = PlanDraft {
        spec: rec.spec.clone(),
        success: rec.success.clone(),
        plan: "1. Write greet.md".into(),
        assertions: Vec::new(),
        reversibility: "reversible".into(),
        tools: vec!["write_file".into()],
        checks: vec![DraftCheck { cmd: "test".into(), args: vec!["-f".into(), "greet.md".into()] }],
    };
    let call = ToolCall { name: "write_file".into(), path: "greet.md".into(), content: Some("hi".into()), params: None };
    let model = ScriptedModel::new(plan, vec![call]);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let mut approved = load(&path);
    approved.approval.as_mut().unwrap().status = "approved".into();
    approved.approval.as_mut().unwrap().by = Some("gui".into());
    save(&path, &approved);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("greet.md")).unwrap(), "hi");
    assert!(!root.join("research.md").exists());
}

#[test]
#[ignore = "calls the official Bright Data MCP server with the local token"]
fn live_public_docs_and_issues() {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".fabric-space");
    let token = tools_contract::secret_value(&home, "brightdata_api_token").expect("brightdata token");
    unsafe { std::env::remove_var("FABR_BRIGHTDATA_MCP_URL") }
    let (_tmp, fabric, root, path) = setup();
    std::fs::write(fabric.join("secrets.toml"), format!("brightdata_api_token = {token:?}\n")).unwrap();
    let mut rec = load(&path);
    rec.state = State::Task;
    rec.step = Some(Step::Plan);
    rec.spec = "Read the public ureq documentation and an upstream issue list.".into();
    rec.success = "research.md cites both public pages.".into();
    save(&path, &rec);
    let plan = PlanDraft {
        spec: rec.spec.clone(),
        success: rec.success.clone(),
        plan: "1. Search public results. 2. Extract the docs page. 3. Extract the issue list. 4. Write research.md.".into(),
        assertions: vec![DraftAssertion { id: "a1".into(), text: "research.md contains https://docs.rs/ureq".into() }],
        reversibility: "reversible".into(),
        tools: vec!["bright_data_search".into(), "bright_data_extract".into()],
        checks: vec![DraftCheck { cmd: "test".into(), args: vec!["-f".into(), "research.md".into()] }],
    };
    let search = serde_json::json!({"query": "Rust ureq HTTP client documentation and GitHub issues"});
    let docs = serde_json::json!({"url": "https://docs.rs/ureq"});
    let issues = serde_json::json!({"url": "https://github.com/algesten/ureq/issues"});
    let calls = vec![
        ToolCall { name: "bright_data_search".into(), path: String::new(), content: None, params: Some(search.clone()) },
        ToolCall { name: "bright_data_extract".into(), path: String::new(), content: None, params: Some(docs.clone()) },
        ToolCall { name: "bright_data_extract".into(), path: String::new(), content: None, params: Some(issues.clone()) },
    ];
    let model = ScriptedModel::new(plan, calls).with_bounds(vec![
        BoundDraft { tool: "bright_data_search".into(), params: search },
        BoundDraft { tool: "bright_data_extract".into(), params: docs },
        BoundDraft { tool: "bright_data_extract".into(), params: issues },
    ]);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    assert_eq!(load(&path).approval.unwrap().status, "pending");
    assert!(!root.join("research.md").exists(), "live retrieval must wait for approval");
    let mut approved = load(&path);
    approved.approval.as_mut().unwrap().status = "approved".into();
    approved.approval.as_mut().unwrap().by = Some("gui".into());
    approved.grants.granted.push(Grant { field: "external_action".into(), scope: "task".into() });
    save(&path, &approved);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let note = std::fs::read_to_string(root.join("research.md")).unwrap();
    assert!(!note.contains(&token));
    assert!(note.contains("source: bright_data"));
    assert!(note.contains("url: https://docs.rs/ureq") || note.contains("https://docs.rs/ureq"));
    let log = load(&path).log;
    assert!(log.contains("bright_data_search"));
    assert!(log.contains("bright_data_extract"));
    assert!(!log.contains(&token));
}
