//! Ignored live calls. They print status words only. Secret values are not printed.

use fabr_agent::connectors::{cartesia, fo, moss, nessie};
use fabr_agent::lifecycle::advance;
use fabr_agent::link::open_reminder;
use fabr_agent::model::{BoundDraft, PlanDraft, ScriptedModel, ToolCall};
use fabr_agent::tools_contract::{self, APIFY_TOKEN, CARTESIA_KEY, FO_KEY, MOSS_PROJECT_ID, MOSS_PROJECT_KEY, NESSIE_KEY};
use fabr_core::taskrec::{self, Grant, State, Step};
use fabr_core::{NodeRef, Workspace};
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap()).join(".fabric-space")
}

fn show(provider: &str, result: Result<String, String>) {
    match result {
        Ok(text) => println!("{provider}: ok {text}"),
        Err(err) => println!("{provider}: {err}"),
    }
}

#[test]
#[ignore = "calls Moss with the local project key"]
fn live_moss_lists_indexes() {
    let space = home();
    let result = moss::list_indexes(&space).map_err(|e| e.message());
    if let Ok(text) = &result {
        assert!(text.starts_with("moss_list indexes="), "{text}");
        assert!(!text.contains("x-project-key"));
    }
    show("moss", result.clone());
    let listed = result.expect("moss listIndexes");
    let names = listed.split_once("names=").map(|(_, names)| names).unwrap_or("");
    let Some(index) = names.split(',').next().filter(|name| !name.is_empty()) else { return };
    let learned = space.join("learned-preferences.json");
    let before = std::fs::read(&learned).ok();
    let retrieved = moss::retrieve(&space, index).map_err(|e| e.message());
    show("moss-retrieve", retrieved.clone());
    let text = retrieved.expect("moss getDocs");
    assert!(text.contains("source=moss"), "{text}");
    assert!(!text.contains("x-project-key"));
    assert_eq!(before, std::fs::read(&learned).ok());
}

#[test]
#[ignore = "calls Fo tools/list and does not invoke a tool"]
fn live_fo_lists_tools() {
    let space = home();
    let Some(key) = tools_contract::secret_value(&space, FO_KEY) else {
        println!("fo: credential missing");
        return;
    };
    let listed = fo::list_tools(&key);
    match &listed {
        Ok(tools) => {
            let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
            println!("fo: ok tools={} names={} allowlist={}", names.len(), names.join(","), fo::ALLOWLIST.len());
            assert!(fo::callable(tools).is_empty());
            assert!(fo::ALLOWLIST.is_empty());
        }
        Err(err) => println!("fo: {err}"),
    }
    listed.expect("fo tools/list");
}

#[test]
#[ignore = "calls Cartesia voices, TTS, and STT"]
fn live_cartesia_speaks_and_transcribes() {
    let space = home();
    let Some(key) = tools_contract::secret_value(&space, CARTESIA_KEY) else {
        println!("cartesia: credential missing");
        return;
    };
    let voices = cartesia::list_voices(&key).map_err(|e| e.message());
    show("cartesia-voices", voices.clone());
    voices.expect("cartesia voices");
    let node = tempfile::tempdir().unwrap();
    let params = serde_json::json!({"transcript": "ok", "voice": first_voice(&key), "path": "speech.wav"});
    let spoken = cartesia::speak(&space, node.path(), &params).map_err(|e| e.message());
    show("cartesia-tts", spoken.clone());
    spoken.expect("cartesia tts");
    assert!(node.path().join("speech.wav").is_file());
    let heard = cartesia::transcribe(&space, node.path(), &serde_json::json!({"path": "speech.wav"})).map_err(|e| e.message());
    show("cartesia-stt", heard.clone());
    heard.expect("cartesia stt");
}

fn first_voice(key: &str) -> String {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(25)).build();
    let resp = match agent.get("https://api.cartesia.ai/voices").set("x-api-key", key).set("cartesia-version", "2024-06-10").call() {
        Ok(resp) => resp,
        Err(ureq::Error::Status(status, _)) => panic!("cartesia voices http {status}"),
        Err(_) => panic!("cartesia voices request failed"),
    };
    let text = resp.into_string().unwrap_or_default();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    let list = value.as_array().or_else(|| value.get("data").and_then(|v| v.as_array())).cloned().unwrap_or_default();
    list.iter().find_map(|item| item.get("id").and_then(|id| id.as_str()).map(str::to_string)).unwrap_or_default()
}

#[test]
#[ignore = "calls the Nessie sandbox"]
fn live_nessie_sandbox_accounts() {
    let space = home();
    let result = nessie::sandbox_accounts(&space).map_err(|e| e.message());
    if let Ok(text) = &result {
        assert!(text.starts_with("nessie sandbox accounts="), "{text}");
        assert!(!text.contains("key="));
    }
    show("nessie", result.clone());
    result.expect("nessie sandbox");
    let learned = space.join("learned-preferences.json");
    let before = std::fs::read(&learned).ok();
    let _ = nessie::sandbox_accounts(&space);
    let after = std::fs::read(&learned).ok();
    assert_eq!(before, after);
}

#[test]
#[ignore = "runs one Apify actor when a token exists"]
fn live_apify_actor() {
    let space = home();
    if tools_contract::secret_value(&space, APIFY_TOKEN).is_none() {
        println!("apify: credential missing");
        return;
    }
    let params = serde_json::json!({"actor": "apify~hello-world", "input": "{}"});
    let result = fabr_agent::connectors::apify::run(&space, &params).map_err(|e| e.message());
    show("apify", result.clone());
    result.expect("apify actor");
}

#[test]
#[ignore = "runs one Apify actor through plan, approval, and execute"]
fn live_apify_task_runs_after_approval() {
    let log =
        drive("apify", "apify_lookup", serde_json::json!({"actor": "apify~hello-world", "input": "{}"}), "external_action", &[APIFY_TOKEN]);
    assert!(log.contains("apify_lookup"), "{log}");
    assert!(log.contains("succeeded"), "{log}");
}

fn stage_space(fabric: &Path, keys: &[&str]) {
    let mut out = String::new();
    for key in keys {
        if let Some(value) = tools_contract::secret_value(&home(), key) {
            out.push_str(key);
            out.push_str(" = ");
            out.push_str(&serde_json::to_string(&value).unwrap_or_default());
            out.push('\n');
        }
    }
    std::fs::write(fabric.join("secrets.toml"), out).unwrap();
}

fn drive(title: &str, tool: &str, params: serde_json::Value, grant: &str, keys: &[&str]) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let cfg = root.join("cfg");
    let cache = root.join("cache");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let ws = Workspace::from_config(&cfg, &cache, Some(root.clone())).unwrap();
    ws.init_node(NodeRef { source: "local".into(), path: String::new() }).unwrap();
    let fabric = tmp.path().join("fabric-space");
    std::fs::create_dir_all(&fabric).unwrap();
    stage_space(&fabric, keys);
    let path = open_reminder(&root, title).unwrap();
    let mut rec = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    rec.state = State::Task;
    rec.step = Some(Step::Plan);
    rec.spec = format!("Use {tool}");
    rec.success = "the tool runs after approval".into();
    let expected = rec.updated_at.clone();
    taskrec::write_record(&path, &rec, Some(&expected)).unwrap();
    let plan = PlanDraft {
        spec: rec.spec.clone(),
        success: rec.success.clone(),
        plan: format!("1. Call {tool}."),
        assertions: Vec::new(),
        reversibility: "reversible".into(),
        tools: vec![tool.into()],
        checks: Vec::new(),
    };
    let model =
        ScriptedModel::new(plan, vec![ToolCall { name: tool.into(), path: String::new(), content: None, params: Some(params.clone()) }])
            .with_bounds(vec![BoundDraft { tool: tool.into(), params }]);
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let mut approved = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(approved.approval.as_ref().unwrap().status, "pending");
    approved.approval.as_mut().unwrap().status = "approved".into();
    approved.approval.as_mut().unwrap().by = Some("gui".into());
    approved.grants.granted.push(Grant { field: grant.into(), scope: "task".into() });
    let expected = approved.updated_at.clone();
    taskrec::write_record(&path, &approved, Some(&expected)).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    advance(&root, &path, &fabric, 10, &model).unwrap();
    let done = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for key in keys {
        if let Some(value) = tools_contract::secret_value(&home(), key)
            && value.len() >= 6
        {
            assert!(!done.log.contains(&value), "{tool} log leaked a secret");
        }
    }
    done.log
}

#[test]
#[ignore = "lists Moss indexes through plan, approval, and execute"]
fn live_moss_task_lists_after_approval() {
    let log = drive("moss", "moss_list", serde_json::json!({}), "external_action", &[MOSS_PROJECT_ID, MOSS_PROJECT_KEY]);
    assert!(log.contains("moss_list"), "{log}");
    assert!(log.contains("succeeded"), "{log}");
}

#[test]
#[ignore = "reads Nessie sandbox accounts through plan, approval, and execute"]
fn live_nessie_task_is_labeled_sandbox() {
    let log = drive("nessie", "nessie_sandbox", serde_json::json!({}), "purchase_finance", &[NESSIE_KEY]);
    assert!(log.contains("nessie sandbox"), "{log}");
    assert!(log.contains("succeeded"), "{log}");
}

#[test]
#[ignore = "synthesizes speech through plan, approval, and execute"]
fn live_cartesia_task_writes_after_approval() {
    let key = tools_contract::secret_value(&home(), CARTESIA_KEY).expect("cartesia key");
    let voice = first_voice(&key);
    assert!(!voice.is_empty());
    let log = drive(
        "speak",
        "cartesia_speak",
        serde_json::json!({"transcript": "ok", "voice": voice, "path": "speech.wav"}),
        "external_action",
        &[CARTESIA_KEY],
    );
    assert!(log.contains("cartesia_speak"), "{log}");
    assert!(log.contains("succeeded"), "{log}");
    assert!(!log.contains(&key));
}

#[test]
fn live_helpers_do_not_touch_visa_without_paths() {
    let space = Path::new(env!("CARGO_MANIFEST_DIR"));
    let _ = space;
    assert!(tools_contract::secret_value(Path::new("/no/such/space"), "visa_cert_path").is_none());
}
