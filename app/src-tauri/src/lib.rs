mod agent;

use agent::AgentHost;
use base64::Engine;
use fabr_core::ops::{Desktop, Done, dispatch, url_allowed};
use fabr_core::workspace::Workspace;
use fabr_core::{AppDirs, ErrorDto, FabrError, client_error};
use tauri::{Manager, RunEvent, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

fn effect(ws: &Workspace, d: Desktop) -> Result<(), FabrError> {
    let io = |e: tauri_plugin_opener::Error| FabrError::Io(e.to_string());
    match d {
        Desktop::OpenPath(r) => tauri_plugin_opener::open_path(ws.local_path(&r)?, None::<&str>).map_err(io),
        Desktop::Reveal(r) => {
            let p = std::path::PathBuf::from(&r.path);
            if !ws.revealable(&p) {
                return Err(FabrError::BadRequest("only the log file can be revealed".into()));
            }
            tauri_plugin_opener::reveal_item_in_dir(p).map_err(io)
        }
        Desktop::OpenUrl(r) => {
            let u = r.url.trim();
            if !url_allowed(u) {
                return Err(FabrError::BadRequest("only http and https links can be opened".into()));
            }
            tauri_plugin_opener::open_url(u, None::<&str>).map_err(io)
        }
    }
}

#[tauri::command(rename_all = "snake_case", async)]
fn call(ws: State<'_, Workspace>, host: State<'_, AgentHost>, name: String, req: serde_json::Value) -> Result<serde_json::Value, ErrorDto> {
    let status = |s: agent::Status| serde_json::to_value(s).map_err(|e| client_error(&name, FabrError::Io(e.to_string())));
    match name.as_str() {
        "fabric_agent_status" => return status(host.status()),
        "fabric_agent_start" => return status(start_agent(&host)),
        "fabric_write_capture" => return speech(&name, write_capture(&req)),
        "fabric_read_speech" => return speech(&name, read_speech(&req)),
        _ => {}
    }
    let out = match dispatch(&ws, &name, req) {
        Ok(Done::Json(v)) => Ok(v),
        Ok(Done::Desktop(d)) => effect(&ws, d).map(|()| serde_json::Value::Null),
        Err(e) => Err(e),
    };
    out.map_err(|e| client_error(&name, e))
}

fn speech(name: &str, result: Result<serde_json::Value, FabrError>) -> Result<serde_json::Value, ErrorDto> {
    result.map_err(|e| client_error(name, e))
}

fn node_of(task_path: &str) -> Result<std::path::PathBuf, FabrError> {
    let path = std::path::Path::new(task_path);
    let tasks = path.parent().ok_or_else(|| FabrError::BadRequest("task path is not a record".into()))?;
    let space = tasks.parent().ok_or_else(|| FabrError::BadRequest("task path is not a record".into()))?;
    let tasks_name = tasks.file_name().and_then(|s| s.to_str());
    let space_name = space.file_name().and_then(|s| s.to_str());
    if tasks_name != Some("tasks") || space_name != Some(".fabric-space") {
        return Err(FabrError::BadRequest("task path is not a record".into()));
    }
    space.parent().map(std::path::Path::to_path_buf).ok_or_else(|| FabrError::BadRequest("task path is not a record".into()))
}

fn write_capture(req: &serde_json::Value) -> Result<serde_json::Value, FabrError> {
    let task = req.get("task_path").and_then(|v| v.as_str()).unwrap_or("");
    let encoded = req.get("audio_base64").and_then(|v| v.as_str()).unwrap_or("");
    let bytes =
        base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| FabrError::BadRequest("audio is not base64".into()))?;
    if bytes.is_empty() || bytes.len() > 5_000_000 {
        return Err(FabrError::BadRequest("audio is empty or too large".into()));
    }
    let node = node_of(task)?;
    fabr_agent::toolset::write_node_bytes(&node, "capture.wav", &bytes).map_err(FabrError::BadRequest)?;
    Ok(serde_json::json!({ "path": "capture.wav" }))
}

fn read_speech(req: &serde_json::Value) -> Result<serde_json::Value, FabrError> {
    let task = req.get("task_path").and_then(|v| v.as_str()).unwrap_or("");
    let text = std::fs::read_to_string(task).map_err(|e| FabrError::Io(e.to_string()))?;
    if !(text.contains("cartesia_speak") && text.contains("succeeded")) {
        return Err(FabrError::BadRequest("speech.wav plays only after an approved cartesia_speak run succeeds".into()));
    }
    let node = node_of(task)?;
    let bytes = fabr_agent::toolset::read_node_bytes(&node, "speech.wav").map_err(FabrError::BadRequest)?;
    Ok(serde_json::json!({ "audio_base64": base64::engine::general_purpose::STANDARD.encode(bytes) }))
}

fn start_agent(host: &AgentHost) -> agent::Status {
    host.start(fabr_core::setup::Ctx::from_env(), |root| AppDirs::resolve().workspace(Some(root)))
}

pub fn run() {
    let log_path = match fabr_core::logging::init_default(cfg!(debug_assertions)) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("fabr: logging disabled: {}", e.message());
            None
        }
    };
    log::info!(
        "fabr app {} starting; log {}",
        env!("CARGO_PKG_VERSION"),
        log_path.as_ref().map_or("(none)".into(), |p| p.display().to_string())
    );
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            match AppDirs::resolve().workspace(AppDirs::env_root()) {
                Ok(ws) => {
                    app.manage(ws);
                    app.manage(AgentHost::default());
                    let host = app.state::<AgentHost>();
                    if fabr_core::setup::settings_read(&fabr_core::setup::Ctx::from_env()).is_ok_and(|r| r.settings.setup.completed) {
                        start_agent(&host);
                    }
                }
                Err(e) => {
                    log::error!("setup failed: {}", e.message());
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                    let where_ = log_path.as_ref().map_or("no log file".into(), |p| format!("Log: {}", p.display()));
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("Fabr could not start:\n\n{}\n\n{where_}", e.message()))
                        .title("Fabr")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![call])
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|handle, event| {
                if let RunEvent::Exit = event
                    && let Some(host) = handle.try_state::<AgentHost>()
                {
                    host.stop();
                }
            })
        });
    if let Err(e) = result {
        log::error!("tauri failed: {e}");
        eprintln!("fabr: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod speech_tests {
    use super::{read_speech, write_capture};

    #[test]
    fn capture_lands_in_the_node_and_speech_waits_for_a_successful_run() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("desk");
        let tasks = node.join(".fabric-space").join("tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        let task = tasks.join("demo.md");
        std::fs::write(&task, "log without a speak result\n").unwrap();
        let req = serde_json::json!({ "task_path": task.to_string_lossy(), "audio_base64": "aGk=" });
        assert_eq!(write_capture(&req).unwrap()["path"], "capture.wav");
        assert_eq!(std::fs::read(node.join("capture.wav")).unwrap(), b"hi");
        assert!(read_speech(&serde_json::json!({ "task_path": task.to_string_lossy() })).is_err());
        std::fs::write(node.join("speech.wav"), b"wav").unwrap();
        std::fs::write(&task, "cartesia_speak succeeded\n").unwrap();
        let played = read_speech(&serde_json::json!({ "task_path": task.to_string_lossy() })).unwrap();
        assert_eq!(played["audio_base64"], "d2F2");
    }
}
