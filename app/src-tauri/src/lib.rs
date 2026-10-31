mod agent;

use agent::AgentHost;
use fabr_core::ops::{Desktop, Done, dispatch, session_command, url_allowed};
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
        Desktop::OpenSession(r) => open_terminal(&session_command(&r)?),
        Desktop::OpenUrl(r) => {
            let u = r.url.trim();
            if !url_allowed(u) {
                return Err(FabrError::BadRequest("only http and https links can be opened".into()));
            }
            tauri_plugin_opener::open_url(u, None::<&str>).map_err(io)
        }
    }
}

fn open_terminal(cmd: &str) -> Result<(), FabrError> {
    use std::process::{Command, Stdio};
    let quiet = |c: &mut Command| c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if cfg!(target_os = "macos") {
        let script = format!("tell application \"Terminal\" to do script \"{}\"", cmd.replace('\\', "\\\\").replace('"', "\\\""));
        let ok = quiet(Command::new("osascript").args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]));
        return if ok { Ok(()) } else { Err(FabrError::Io("Could not open Terminal. Copy the command and run it yourself.".into())) };
    }
    let hold = format!("{cmd}; exec \"${{SHELL:-sh}}\"");
    let tries: [(&str, Vec<&str>); 4] = [
        ("x-terminal-emulator", vec!["-e", "sh", "-c", &hold]),
        ("gnome-terminal", vec!["--", "sh", "-c", &hold]),
        ("konsole", vec!["-e", "sh", "-c", &hold]),
        ("xterm", vec!["-e", "sh", "-c", &hold]),
    ];
    for (bin, args) in tries {
        if Command::new(bin).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return Ok(());
        }
    }
    Err(FabrError::Io("No terminal program was found. Copy the command and run it yourself.".into()))
}

#[tauri::command(rename_all = "snake_case", async)]
fn call(ws: State<'_, Workspace>, host: State<'_, AgentHost>, name: String, req: serde_json::Value) -> Result<serde_json::Value, ErrorDto> {
    let status = |s: agent::Status| serde_json::to_value(s).map_err(|e| client_error(&name, FabrError::Io(e.to_string())));
    match name.as_str() {
        "fabric_agent_status" => return status(host.status()),
        "fabric_agent_start" => return status(start_agent(&host)),
        _ => {}
    }
    let out = match dispatch(&ws, &name, req) {
        Ok(Done::Json(v)) => Ok(v),
        Ok(Done::Desktop(d)) => effect(&ws, d).map(|()| serde_json::Value::Null),
        Err(e) => Err(e),
    };
    out.map_err(|e| client_error(&name, e))
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
