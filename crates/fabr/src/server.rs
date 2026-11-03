use fabr_core::workspace::Workspace;
use fabr_core::{Done, ErrorDto, FabrError, OPS, Verb, client_error, dispatch};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{Cursor, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server};

pub const MAX_BODY: usize = 1 << 20;

#[cfg(fabr_ui)]
static EMBEDDED: include_dir::Dir<'static> = include_dir::include_dir!("$FABR_UI_DIR");

type Resp = Response<Cursor<Vec<u8>>>;

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn json<T: Serialize>(status: u16, body: &T) -> Resp {
    let data = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    Response::from_data(data)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("Cache-Control", "no-store"))
}

fn error(code: u16, message: &str) -> Resp {
    json(code, &ErrorDto { code, message: message.to_string(), node: None })
}

fn api_error(op: &str, e: FabrError) -> Resp {
    let dto = client_error(op, e);
    json(dto.code, &dto)
}

fn get_header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
}

fn host_part(v: &str) -> &str {
    if let Some(rest) = v.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("");
    }
    match v.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => v,
    }
}

pub fn host_is_loopback(host: &str) -> bool {
    let h = host_part(host);
    h.eq_ignore_ascii_case("localhost") || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub fn origin_matches(origin: &str, host: &str) -> bool {
    let rest = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://"));
    rest.is_some_and(|r| r.trim_end_matches('/').eq_ignore_ascii_case(host))
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub enum Ui {
    Dir(PathBuf),
    Embedded,
}

impl Ui {
    pub fn new(dir: Option<PathBuf>) -> Ui {
        match dir {
            Some(d) => Ui::Dir(d),
            None => Ui::Embedded,
        }
    }

    fn get(&self, rel: &str) -> Option<Vec<u8>> {
        match self {
            Ui::Dir(d) => {
                let base = d.canonicalize().ok()?;
                let f = base.join(rel).canonicalize().ok()?;
                if !f.starts_with(&base) || !f.is_file() {
                    return None;
                }
                std::fs::read(f).ok()
            }
            Ui::Embedded => embedded(rel),
        }
    }
}

#[cfg(fabr_ui)]
fn embedded(rel: &str) -> Option<Vec<u8>> {
    EMBEDDED.get_file(rel).map(|f| f.contents().to_vec())
}

#[cfg(not(fabr_ui))]
fn embedded(_rel: &str) -> Option<Vec<u8>> {
    None
}

fn serve_static(ui: &Ui, path: &str) -> Resp {
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|s| s == ".." || s.is_empty() || s.starts_with('.')) || rel.contains('\\') {
        return error(404, "not found");
    }
    match ui.get(rel) {
        Some(data) => Response::from_data(data).with_header(header("Content-Type", content_type(rel))),
        None if rel == "index.html" => error(404, "the UI is not built: run `npm run build` in app/ or pass --ui-dir"),
        None => error(404, "not found"),
    }
}

fn read_body<T: DeserializeOwned>(req: &mut Request) -> Result<T, Resp> {
    let ct = get_header(req, "Content-Type").unwrap_or("");
    if !ct.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json") {
        return Err(error(415, "expected application/json"));
    }
    if req.body_length().is_some_and(|n| n > MAX_BODY) {
        return Err(error(413, "request too large"));
    }
    let mut buf = Vec::new();
    if req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut buf).is_err() {
        return Err(error(400, "could not read the request body"));
    }
    if buf.len() > MAX_BODY {
        return Err(error(413, "request too large"));
    }
    serde_json::from_slice(&buf).map_err(|e| error(400, &format!("bad request body: {e}")))
}

fn query_object(query: &str) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in query.split('&').filter_map(|kv| kv.split_once('=')) {
        let on = v == "1" || v == "true";
        if on || !m.contains_key(k) {
            m.insert(k.to_string(), serde_json::Value::Bool(on));
        }
    }
    serde_json::Value::Object(m)
}

fn handle_lantern(path: &str, req: &mut Request) -> Option<Resp> {
    if path == "/lantern/graph" {
        if req.method() != &Method::Post {
            return Some(error(405, "method not allowed"));
        }
        #[derive(serde::Deserialize)]
        struct GraphReq {
            task_id: String,
            run_id: Option<String>,
            folder: Option<PathBuf>,
        }
        let body: GraphReq = match read_body(req) {
            Ok(b) => b,
            Err(r) => return Some(r),
        };
        let space_dir = crate::lantern_cli::resolve_space_dir(body.folder);
        let run_id = match crate::lantern_cli::resolve_run_id(&space_dir, &body.task_id, body.run_id) {
            Ok(r) => r,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let run_log = match fabr_agent::lantern::RunLog::new(&space_dir, &body.task_id, &run_id) {
            Ok(l) => l,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let events = match run_log.read_all() {
            Ok(ev) => ev,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let graph = fabr_agent::lantern::LanternGraph::from_events(&events);
        return Some(json(200, &graph));
    }

    if let Some(rest) = path.strip_prefix("/lantern/")
        && let Some(task_id) = rest.strip_suffix("/graph")
    {
        if req.method() != &Method::Get {
            return Some(error(405, "method not allowed"));
        }
        let space_dir = crate::lantern_cli::resolve_space_dir(None);
        let run_id = match crate::lantern_cli::resolve_run_id(&space_dir, task_id, None) {
            Ok(r) => r,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let run_log = match fabr_agent::lantern::RunLog::new(&space_dir, task_id, &run_id) {
            Ok(l) => l,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let events = match run_log.read_all() {
            Ok(ev) => ev,
            Err(e) => return Some(api_error("lantern_graph", e)),
        };
        let graph = fabr_agent::lantern::LanternGraph::from_events(&events);
        return Some(json(200, &graph));
    }

    if path == "/lantern/steer" {
        if req.method() != &Method::Post {
            return Some(error(405, "method not allowed"));
        }
        #[derive(serde::Deserialize)]
        struct SteerReq {
            task_id: String,
            run_id: Option<String>,
            folder: Option<PathBuf>,
            action: fabr_agent::lantern::SteeringAction,
            by: Option<String>,
        }
        let body: SteerReq = match read_body(req) {
            Ok(b) => b,
            Err(r) => return Some(r),
        };
        let space_dir = crate::lantern_cli::resolve_space_dir(body.folder);
        let run_id = match crate::lantern_cli::resolve_run_id(&space_dir, &body.task_id, body.run_id) {
            Ok(r) => r,
            Err(e) => return Some(api_error("lantern_steer", e)),
        };
        let run_log = match fabr_agent::lantern::RunLog::new(&space_dir, &body.task_id, &run_id) {
            Ok(l) => l,
            Err(e) => return Some(api_error("lantern_steer", e)),
        };
        let events = match run_log.read_all() {
            Ok(ev) => ev,
            Err(e) => return Some(api_error("lantern_steer", e)),
        };
        let mut graph = fabr_agent::lantern::LanternGraph::from_events(&events);
        let effects_log = fabr_agent::lantern::EffectsLog::new(run_log.dir());

        let mut state = if let Ok(Some(cp)) = fabr_agent::lantern::Checkpoint::load_latest(run_log.dir()) {
            cp.exploration_state
        } else {
            fabr_agent::lantern::ExplorationState::new(fabr_agent::lantern::ExplorationBudget::default())
        };

        let report = match fabr_agent::lantern::apply_steering(
            &mut state,
            &mut graph,
            &run_log,
            &effects_log,
            &body.task_id,
            body.action,
            body.by.as_deref().unwrap_or("user"),
        ) {
            Ok(rep) => rep,
            Err(e) => return Some(api_error("lantern_steer", e)),
        };
        return Some(json(200, &report));
    }

    None
}

pub fn route(ws: &Workspace, ui: &Ui, req: &mut Request) -> Resp {
    let Some(host) = get_header(req, "Host").map(str::to_string) else {
        return error(403, "missing Host header");
    };
    if !host_is_loopback(&host) {
        return error(403, "Host must be a loopback address");
    }
    if let Some(origin) = get_header(req, "Origin")
        && !origin_matches(origin, &host)
    {
        return error(403, "cross-origin request refused");
    }
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    if let Some(resp) = handle_lantern(path, req) {
        return resp;
    }
    let Some(name) = path.strip_prefix("/api/") else {
        return match req.method() {
            Method::Get | Method::Head => serve_static(ui, path),
            _ => error(405, "method not allowed"),
        };
    };
    let Some(op) = OPS.iter().find(|o| o.http && o.name == name) else {
        return error(404, "no such route");
    };
    let body = match (op.verb, req.method()) {
        (Verb::Get, Method::Get) => query_object(query),
        (Verb::Post, Method::Post) => match read_body::<serde_json::Value>(req) {
            Ok(b) => b,
            Err(r) => return r,
        },
        _ => return error(405, "method not allowed"),
    };
    match dispatch(ws, op.name, body) {
        Ok(Done::Json(v)) => json(200, &v),
        Ok(Done::Desktop(_)) => error(501, &format!("{} is only available in the desktop app", op.name)),
        Err(e) => api_error(op.name, e),
    }
}

pub fn bind(port: u16) -> std::io::Result<(Arc<Server>, u16)> {
    let server = Server::http(("127.0.0.1", port)).map_err(std::io::Error::other)?;
    let port = server.server_addr().to_ip().map_or(port, |a| a.port());
    Ok((Arc::new(server), port))
}

pub fn is_long_op(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    matches!(path, "/api/fabric_check" | "/api/fabric_telegram_link")
}

pub fn spawn_workers(server: Arc<Server>, ws: Arc<Workspace>, ui: Arc<Ui>, threads: usize) -> Vec<std::thread::JoinHandle<()>> {
    (0..threads)
        .map(|_| {
            let (server, ws, ui) = (server.clone(), ws.clone(), ui.clone());
            std::thread::spawn(move || {
                for mut req in server.incoming_requests() {
                    if is_long_op(req.url()) {
                        let (ws, ui) = (ws.clone(), ui.clone());
                        std::thread::spawn(move || {
                            let resp = route(&ws, &ui, &mut req);
                            let _ = req.respond(resp);
                        });
                        continue;
                    }
                    let resp = route(&ws, &ui, &mut req);
                    let _ = req.respond(resp);
                }
            })
        })
        .collect()
}

pub fn open_browser(url: &str) {
    let prog = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(prog).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

pub fn ui_dir_ok(p: &Path) -> bool {
    p.join("index.html").is_file()
}

#[cfg(test)]
mod long_op_tests {
    use super::is_long_op;

    #[test]
    fn only_network_ops_get_their_own_thread() {
        assert!(is_long_op("/api/fabric_check"));
        assert!(is_long_op("/api/fabric_telegram_link?x=1"));
        assert!(!is_long_op("/api/fabric_secrets_status"));
        assert!(!is_long_op("/"));
    }
}
