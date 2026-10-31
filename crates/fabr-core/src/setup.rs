use crate::error::FabrError;
use crate::paths::expand_home;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub const MODELS: [&str; 5] = ["", "sonnet", "opus", "haiku", "fable"];
pub const EFFORTS: [&str; 6] = ["", "low", "medium", "high", "xhigh", "max"];
const TELEGRAM_URL: &str = "https://api.telegram.org";
const NO_MESSAGE: &str = "No message arrived in time. Open your bot in Telegram and send it any text.";
const NO_CONFIRM: &str = "Confirm was not tapped in time. Tap Confirm under the message from your bot.";

#[derive(Debug, Clone)]
pub struct Ctx {
    pub home: PathBuf,
    pub telegram_url: String,
    pub poll_s: u64,
}

impl Ctx {
    pub fn new(home: impl Into<PathBuf>) -> Ctx {
        Ctx { home: home.into(), telegram_url: TELEGRAM_URL.into(), poll_s: 10 }
    }

    pub fn from_env() -> Ctx {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = env("FABR_HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let mut c = Ctx::new(home);
        if let Some(u) = env("FABR_TELEGRAM_URL") {
            c.telegram_url = u;
        }
        c
    }

    pub fn space(&self) -> PathBuf {
        self.home.join(".fabric-space")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.space().join("settings.toml")
    }
    pub fn secrets_path(&self) -> PathBuf {
        self.space().join("secrets.toml")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupState {
    #[serde(default)]
    pub completed_steps: Vec<String>,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMode {
    #[default]
    Full,
    ReadOnly,
    Off,
}

impl ExecuteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ExecuteMode::Full => "full",
            ExecuteMode::ReadOnly => "read_only",
            ExecuteMode::Off => "off",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub checks: Vec<String>,
}

impl Capabilities {
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.checks.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabricSettings {
    pub default_folder: String,
    pub model: String,
    pub telegram_user_id: Option<i64>,
    pub auto_mode: bool,
    pub question_budget: u32,
    pub heartbeat_interval_s: u32,
    pub setup: SetupState,
    #[serde(default)]
    pub execute: ExecuteMode,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub claude_bin: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub max_budget_usd: f64,
    #[serde(default = "default_execute_timeout_s")]
    pub execute_timeout_s: u32,
}

pub const EXECUTE_TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 60..=14400;

fn default_execute_timeout_s() -> u32 {
    1800
}

impl FabricSettings {
    pub fn defaults(home: &Path) -> FabricSettings {
        FabricSettings {
            default_folder: home.to_string_lossy().into_owned(),
            model: MODELS[0].into(),
            telegram_user_id: None,
            auto_mode: false,
            question_budget: 5,
            heartbeat_interval_s: 10,
            setup: SetupState { completed_steps: vec![], completed: false },
            execute: ExecuteMode::Full,
            capabilities: Capabilities::default(),
            claude_bin: String::new(),
            effort: String::new(),
            max_budget_usd: 0.0,
            execute_timeout_s: default_execute_timeout_s(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsRead {
    pub exists: bool,
    pub settings: FabricSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsWriteReq {
    pub settings: FabricSettings,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SecretsWriteReq {
    #[serde(default)]
    pub telegram_bot_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretsStatus {
    pub telegram_bot_token: bool,
    #[serde(default)]
    pub telegram_bot_token_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckReq {
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LinkReq {
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> Result<(), FabrError> {
    let parent = path.parent().ok_or_else(|| FabrError::Io("bad path".into()))?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    tmp.write_all(bytes)?;
    tmp.persist(path).map_err(|e| FabrError::Io(e.to_string()))?;
    Ok(())
}

fn load_doc(path: &Path) -> Result<Option<DocumentMut>, FabrError> {
    match std::fs::read_to_string(path) {
        Ok(t) => t.parse::<DocumentMut>().map(Some).map_err(|e| FabrError::Unprocessable(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn settings_read(ctx: &Ctx) -> Result<SettingsRead, FabrError> {
    let mut s = FabricSettings::defaults(&ctx.home);
    let Some(doc) = load_doc(&ctx.settings_path())? else { return Ok(SettingsRead { exists: false, settings: s }) };
    if let Some(v) = doc.get("default_folder").and_then(Item::as_str) {
        s.default_folder = expand_home(v, &ctx.home).to_string_lossy().into_owned();
    }
    if let Some(v) = doc.get("model").and_then(Item::as_str) {
        s.model = v.into();
    }
    if let Some(v) = doc.get("claude_bin").and_then(Item::as_str) {
        s.claude_bin = v.into();
    }
    if let Some(v) = doc.get("effort").and_then(Item::as_str) {
        s.effort = v.into();
    }
    if let Some(n) = doc.get("max_budget_usd").and_then(|i| i.as_float().or_else(|| i.as_integer().map(|n| n as f64))) {
        s.max_budget_usd = if n.is_finite() && n > 0.0 { n } else { 0.0 };
    }
    s.telegram_user_id = doc.get("telegram_user_id").and_then(Item::as_integer);
    if let Some(v) = doc.get("auto_mode").and_then(Item::as_bool) {
        s.auto_mode = v;
    }
    let int = |k: &str, d: u32| doc.get(k).and_then(Item::as_integer).map_or(d, |n| n.clamp(0, u32::MAX as i64) as u32);
    s.question_budget = int("question_budget", s.question_budget);
    s.heartbeat_interval_s = int("heartbeat_interval_s", s.heartbeat_interval_s);
    s.execute_timeout_s = int("execute_timeout_s", s.execute_timeout_s).clamp(*EXECUTE_TIMEOUT_RANGE.start(), *EXECUTE_TIMEOUT_RANGE.end());
    if let Some(t) = doc.get("setup").and_then(Item::as_table_like) {
        if let Some(a) = t.get("completed_steps").and_then(Item::as_array) {
            s.setup.completed_steps = a.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        }
        if let Some(b) = t.get("completed").and_then(Item::as_bool) {
            s.setup.completed = b;
        }
    }
    if let Some(m) = doc.get("execute").and_then(Item::as_str) {
        s.execute = match m {
            "read_only" => ExecuteMode::ReadOnly,
            "off" => ExecuteMode::Off,
            _ => ExecuteMode::Full,
        };
    }
    if let Some(t) = doc.get("capabilities").and_then(Item::as_table_like) {
        let list = |k: &str| -> Vec<String> {
            t.get(k).and_then(Item::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
        };
        s.capabilities = Capabilities { tools: list("tools"), checks: list("checks") };
    }
    Ok(SettingsRead { exists: true, settings: s })
}

pub fn settings_write(ctx: &Ctx, req: SettingsWriteReq) -> Result<SettingsRead, FabrError> {
    let s = req.settings;
    if !valid_model(&s.model) {
        return Err(FabrError::BadRequest(format!("model must be empty, one of {}, or a full claude- model name", MODELS[1..].join(", "))));
    }
    if !EFFORTS.contains(&s.effort.as_str()) {
        return Err(FabrError::BadRequest(format!("effort must be empty or one of {}", EFFORTS[1..].join(", "))));
    }
    if !s.max_budget_usd.is_finite() || s.max_budget_usd < 0.0 {
        return Err(FabrError::BadRequest("max_budget_usd must be 0 (no cap) or a positive amount".into()));
    }
    if !EXECUTE_TIMEOUT_RANGE.contains(&s.execute_timeout_s) {
        return Err(FabrError::BadRequest(format!(
            "execute_timeout_s must be between {} and {}",
            EXECUTE_TIMEOUT_RANGE.start(),
            EXECUTE_TIMEOUT_RANGE.end()
        )));
    }
    let path = ctx.settings_path();
    let mut doc = load_doc(&path)?.unwrap_or_default();
    doc["default_folder"] = value(expand_home(&s.default_folder, &ctx.home).to_string_lossy().into_owned());
    doc["model"] = value(s.model);
    doc["claude_bin"] = value(s.claude_bin);
    doc["effort"] = value(s.effort);
    doc["max_budget_usd"] = value(s.max_budget_usd);
    doc["execute_timeout_s"] = value(s.execute_timeout_s as i64);
    match s.telegram_user_id {
        Some(id) => doc["telegram_user_id"] = value(id),
        None => {
            doc.remove("telegram_user_id");
        }
    }
    doc["auto_mode"] = value(s.auto_mode);
    doc["question_budget"] = value(s.question_budget as i64);
    doc["heartbeat_interval_s"] = value(s.heartbeat_interval_s as i64);
    if !doc.get("setup").is_some_and(Item::is_table) {
        doc["setup"] = Item::Table(Table::new());
    }
    let setup = &mut doc["setup"];
    setup["completed_steps"] = value(s.setup.completed_steps.iter().map(String::as_str).collect::<Array>());
    setup["completed"] = value(s.setup.completed);
    doc["execute"] = value(s.execute.as_str());
    if !s.capabilities.is_empty() {
        if !doc.get("capabilities").is_some_and(Item::is_table) {
            doc["capabilities"] = Item::Table(Table::new());
        }
        let caps = &mut doc["capabilities"];
        caps["tools"] = value(s.capabilities.tools.iter().map(String::as_str).collect::<Array>());
        caps["checks"] = value(s.capabilities.checks.iter().map(String::as_str).collect::<Array>());
    }
    atomic_write(&path, doc.to_string().as_bytes(), false)?;
    settings_read(ctx)
}

pub fn valid_model(model: &str) -> bool {
    MODELS.contains(&model) || (model.starts_with("claude-") && model.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'))
}

pub fn read_secret(ctx: &Ctx, key: &str) -> Option<String> {
    let doc = load_doc(&ctx.secrets_path()).ok().flatten()?;
    doc.get(key).and_then(Item::as_str).filter(|v| !v.is_empty()).map(String::from)
}

pub fn secrets_write(ctx: &Ctx, req: SecretsWriteReq) -> Result<SecretsStatus, FabrError> {
    let path = ctx.secrets_path();
    let mut doc = load_doc(&path)?.unwrap_or_default();
    if let Some(v) = req.telegram_bot_token.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        doc["telegram_bot_token"] = value(v);
    }
    atomic_write(&path, doc.to_string().as_bytes(), true)?;
    Ok(secrets_status(ctx))
}

pub fn secrets_status(ctx: &Ctx) -> SecretsStatus {
    let token = read_secret(ctx, "telegram_bot_token");
    SecretsStatus { telegram_bot_token: token.is_some(), telegram_bot_token_fingerprint: token.as_deref().map(crate::redact::fingerprint) }
}

fn fail(reason: &str) -> Value {
    json!({ "ok": false, "reason": reason })
}

enum Reply {
    Json(u16, Value),
    Down,
}

fn send(req: ureq::Request, body: Option<&Value>) -> Reply {
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    match res {
        Ok(r) => {
            let code = r.status();
            Reply::Json(code, r.into_json().unwrap_or(Value::Null))
        }
        Err(ureq::Error::Status(code, r)) => Reply::Json(code, r.into_json().unwrap_or(Value::Null)),
        Err(_) => Reply::Down,
    }
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(secs)).build()
}

const TELEGRAM_DOWN: &str = "Could not reach Telegram. Check the internet connection and try again.";

pub fn check_folder(ctx: &Ctx, path: &str) -> Value {
    let space = ctx.space();
    if let Err(e) = std::fs::create_dir_all(&space) {
        return fail(&format!("Could not create {}: {}.", space.display(), e.kind()));
    }
    let p = expand_home(path, &ctx.home);
    if !p.exists() {
        return fail("That folder does not exist.");
    }
    if !p.is_dir() {
        return fail("That path is a file, not a folder.");
    }
    match tempfile::Builder::new().prefix(".fabr-check-").tempfile_in(&p) {
        Ok(_) => json!({ "ok": true }),
        Err(_) => fail("Fabr cannot write to that folder. Pick one you own."),
    }
}

const CLAUDE_MISSING: &str = "Claude Code is not installed. Install it from https://claude.com/claude-code, then press Check again.";
const NOT_SIGNED_IN: &str = "Claude Code is installed but not signed in. Run claude in a terminal and sign in, then press Check again.";
const CLAUDE_TIMEOUT_S: u64 = 10;

fn is_executable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        m.is_file()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| is_executable(p))
}

pub fn check_claude(ctx: &Ctx) -> Value {
    let bin = settings_read(ctx).map(|r| r.settings.claude_bin).unwrap_or_default();
    check_claude_at(ctx, &bin)
}

pub fn check_claude_at(ctx: &Ctx, bin: &str) -> Value {
    check_claude_timeout(ctx, bin, Duration::from_secs(CLAUDE_TIMEOUT_S))
}

fn check_claude_timeout(ctx: &Ctx, bin: &str, limit: Duration) -> Value {
    let bin = bin.trim();
    let path = if bin.is_empty() {
        match on_path("claude") {
            Some(p) => p,
            None => return fail(CLAUDE_MISSING),
        }
    } else {
        let p = expand_home(bin, &ctx.home);
        if p.components().count() > 1 || bin.starts_with('.') {
            if !p.exists() {
                return fail(&format!("Nothing found at {}. Check the path, or clear it to use claude from PATH.", p.display()));
            }
            p
        } else {
            match on_path(bin) {
                Some(p) => p,
                None => return fail(&format!("{bin} was not found on PATH. Enter its full path.")),
            }
        }
    };
    if !is_executable(&path) {
        return fail(&format!("{} is not an executable file.", path.display()));
    }
    let (ok, out) = match run_capture(&path, "--version", limit) {
        Ok(r) => r,
        Err(RunErr::Spawn(e)) => return fail(&format!("Could not run {}: {}.", path.display(), e)),
        Err(RunErr::Timeout) => {
            return fail(&format!("{} --version did not answer in {} seconds. Press Check again.", path.display(), limit.as_secs()));
        }
    };
    let version = out.lines().next().unwrap_or("").trim().to_string();
    if !ok || version.is_empty() {
        return fail(&format!("{} --version failed. Is this the Claude Code program?", path.display()));
    }
    let mut r = json!({ "ok": true, "version": version, "path": path.to_string_lossy() });
    if let Ok((_, text)) = run_capture(&path, "auth status", limit)
        && let Some((logged_in, auth)) = parse_auth(&text)
    {
        if !logged_in {
            return fail(NOT_SIGNED_IN);
        }
        r["logged_in"] = json!(true);
        if let Some(a) = auth {
            r["auth"] = json!(a);
        }
    }
    r
}

enum RunErr {
    Spawn(String),
    Timeout,
}

fn run_capture(path: &Path, args: &str, limit: Duration) -> Result<(bool, String), RunErr> {
    let mut cmd = std::process::Command::new(path);
    cmd.args(args.split(' ')).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let mut tries = 0;
    let mut child = loop {
        match cmd.spawn() {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(RunErr::Spawn(e.kind().to_string())),
        }
    };
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunErr::Timeout);
            }
            Err(e) => return Err(RunErr::Spawn(e.kind().to_string())),
        }
    };
    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut so, &mut out);
    }
    Ok((status.success(), out))
}

fn parse_auth(text: &str) -> Option<(bool, Option<String>)> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let logged_in = v.get("loggedIn")?.as_bool()?;
    if !logged_in {
        return Some((false, None));
    }
    let method = v["authMethod"].as_str().unwrap_or("");
    let plan = v["subscriptionType"].as_str().unwrap_or("");
    let label = match (method, plan) {
        ("", "") => String::new(),
        ("", p) => p.to_string(),
        (m, "") if m.contains("api") || m.contains("key") => "API key".to_string(),
        (m, "") => m.to_string(),
        (m, p) => format!("{m}, {p}"),
    };
    Some((true, Some(label).filter(|l| !l.is_empty())))
}

fn tg(ctx: &Ctx, token: &str, method: &str) -> String {
    format!("{}/bot{token}/{method}", ctx.telegram_url.trim_end_matches('/'))
}

pub fn check_telegram_token(ctx: &Ctx) -> Value {
    let Some(token) = read_secret(ctx, "telegram_bot_token") else { return fail("No bot token is saved yet. Enter it first.") };
    match send(agent(15).get(&tg(ctx, &token, "getMe")), None) {
        Reply::Json(200, v) if v["ok"].as_bool() == Some(true) => {
            let r = &v["result"];
            let name = r["username"].as_str().or_else(|| r["first_name"].as_str()).unwrap_or("");
            json!({ "ok": true, "bot_name": name })
        }
        Reply::Json(401 | 404, _) => fail("Telegram did not accept the bot token. Copy it again from BotFather."),
        Reply::Json(c, _) => fail(&format!("Telegram answered with HTTP {c}. Try again in a moment.")),
        Reply::Down => fail(TELEGRAM_DOWN),
    }
}

pub const DEFAULT_LINK_TIMEOUT_S: u64 = 120;

pub fn telegram_link(ctx: &Ctx, timeout_s: u64) -> Value {
    let Some(token) = read_secret(ctx, "telegram_bot_token") else { return fail("No bot token is saved yet. Enter it first.") };
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    let mut offset: i64 = 0;
    let mut user: Option<(i64, i64)> = None;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return fail(if user.is_some() { NO_CONFIRM } else { NO_MESSAGE });
        }
        let wait = ctx.poll_s.min(left.as_secs().max(1));
        let body = json!({ "offset": offset, "timeout": wait, "allowed_updates": ["message", "callback_query"] });
        let res = send(agent(wait + 10).post(&tg(ctx, &token, "getUpdates")), Some(&body));
        let updates = match res {
            Reply::Json(200, v) => v["result"].as_array().cloned().unwrap_or_default(),
            Reply::Json(401 | 404, _) => return fail("Telegram did not accept the bot token. Copy it again from BotFather."),
            Reply::Json(409, _) => return fail("Another program is already reading this bot's messages. Stop it and try again."),
            _ => vec![],
        };
        if updates.is_empty() && ctx.poll_s == 0 {
            std::thread::sleep(Duration::from_millis(50));
        }
        for u in updates {
            offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
            match user {
                None => {
                    let m = &u["message"];
                    let (Some(uid), Some(chat)) = (m["from"]["id"].as_i64(), m["chat"]["id"].as_i64()) else { continue };
                    let body = json!({
                        "chat_id": chat,
                        "text": "connected",
                        "reply_markup": { "inline_keyboard": [[{ "text": "Confirm", "callback_data": "confirm" }]] },
                    });
                    match send(agent(15).post(&tg(ctx, &token, "sendMessage")), Some(&body)) {
                        Reply::Json(200, _) => user = Some((uid, chat)),
                        Reply::Json(c, _) => return fail(&format!("Telegram would not send the reply (HTTP {c}).")),
                        Reply::Down => return fail(TELEGRAM_DOWN),
                    }
                }
                Some((uid, _)) => {
                    let cb = &u["callback_query"];
                    if cb["from"]["id"].as_i64() == Some(uid) && cb["data"].as_str() == Some("confirm") {
                        let body = json!({ "callback_query_id": cb["id"] });
                        let _ = send(agent(15).post(&tg(ctx, &token, "answerCallbackQuery")), Some(&body));
                        return json!({ "ok": true, "user_id": uid });
                    }
                }
            }
        }
    }
}

pub fn check(ctx: &Ctx, req: CheckReq) -> Result<Value, FabrError> {
    let need = |v: &Option<String>, what: &str| v.clone().ok_or_else(|| FabrError::BadRequest(format!("{what} is required")));
    Ok(match req.kind.as_str() {
        "folder" => check_folder(ctx, &need(&req.path, "path")?),
        "claude" => match req.path.as_deref() {
            Some(p) => check_claude_at(ctx, p),
            None => check_claude(ctx),
        },
        "telegram_token" => check_telegram_token(ctx),
        k => return Err(FabrError::BadRequest(format!("unknown check {k:?}"))),
    })
}

pub fn link(ctx: &Ctx, req: LinkReq) -> Value {
    telegram_link(ctx, req.timeout_s.unwrap_or(DEFAULT_LINK_TIMEOUT_S))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(String, Value)>>>;

    struct Fake {
        url: String,
        log: Log,
        _stop: Arc<std::sync::atomic::AtomicBool>,
    }

    fn fake(updates: Vec<Value>) -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let log: Log = Arc::default();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (l, s) = (log.clone(), stop.clone());
        let mut queue: VecDeque<Value> = updates.into();
        std::thread::spawn(move || {
            while !s.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(Some(mut rq)) = server.recv_timeout(Duration::from_millis(100)) else { continue };
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(rq.as_reader(), &mut text);
                let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let path = rq.url().to_string();
                l.lock().unwrap().push((path.clone(), json!({ "body": body })));
                let (code, out) = if path.ends_with("/getMe") {
                    (200, json!({ "ok": true, "result": { "first_name": "Fabr Bot", "username": "fabr_test_bot" } }))
                } else if path.ends_with("/getUpdates") {
                    let next = queue.pop_front().map(|u| vec![u]).unwrap_or_default();
                    (200, json!({ "ok": true, "result": next }))
                } else {
                    (200, json!({ "ok": true, "result": true }))
                };
                let resp = tiny_http::Response::from_string(out.to_string())
                    .with_status_code(code)
                    .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
                let _ = rq.respond(resp);
            }
        });
        Fake { url, log, _stop: stop }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self._stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn ctx(f: &Fake) -> (tempfile::TempDir, Ctx) {
        let d = tempfile::tempdir().unwrap();
        let mut c = Ctx::new(d.path());
        c.telegram_url = f.url.clone();
        c.poll_s = 0;
        secrets_write(&c, SecretsWriteReq { telegram_bot_token: Some("123456:FAKE-TOKEN".into()) }).unwrap();
        (d, c)
    }

    fn msg(id: i64, uid: i64) -> Value {
        json!({ "update_id": id, "message": { "from": { "id": uid }, "chat": { "id": uid }, "text": "hi" } })
    }
    fn tap(id: i64, uid: i64, data: &str) -> Value {
        json!({ "update_id": id, "callback_query": { "id": "cb1", "from": { "id": uid }, "data": data } })
    }

    #[test]
    fn settings_round_trip_keeps_unknown_keys() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let r = settings_read(&c).unwrap();
        assert!(!r.exists);
        assert_eq!(r.settings.model, "");
        assert_eq!(r.settings.question_budget, 5);
        std::fs::create_dir_all(c.space()).unwrap();
        std::fs::write(c.settings_path(), "future_key = \"x\"\n[setup]\nextra = 1\n").unwrap();
        let mut s = settings_read(&c).unwrap().settings;
        s.telegram_user_id = Some(42);
        s.setup.completed_steps = vec!["folder".into(), "api_key".into(), "model".into()];
        s.setup.completed = true;
        s.model = "claude-haiku-5-5".into();
        s.claude_bin = "/opt/claude".into();
        s.effort = "high".into();
        s.max_budget_usd = 2.5;
        s.execute_timeout_s = 600;
        let w = settings_write(&c, SettingsWriteReq { settings: s.clone() }).unwrap();
        assert!(w.exists);
        assert_eq!(w.settings, s);
        let text = std::fs::read_to_string(c.settings_path()).unwrap();
        assert!(text.contains("future_key = \"x\"") && text.contains("extra = 1"));
        assert!(text.contains("max_budget_usd = 2.5") && text.contains("execute_timeout_s = 600"));
        s.telegram_user_id = None;
        settings_write(&c, SettingsWriteReq { settings: s }).unwrap();
        assert_eq!(settings_read(&c).unwrap().settings.telegram_user_id, None);
        let mut bad = settings_read(&c).unwrap().settings;
        bad.model = "gpt".into();
        assert!(settings_write(&c, SettingsWriteReq { settings: bad }).is_err());
    }

    #[test]
    fn secrets_are_private_and_never_reported() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let st = secrets_status(&c);
        assert!(!st.telegram_bot_token && st.telegram_bot_token_fingerprint.is_none());
        std::fs::create_dir_all(c.space()).unwrap();
        std::fs::write(c.secrets_path(), "anthropic_api_key = \"sk-ant-OLD\"\n").unwrap();
        let st = secrets_write(&c, SecretsWriteReq { telegram_bot_token: Some("123456:FAKE-TOKEN".into()) }).unwrap();
        assert!(st.telegram_bot_token);
        assert_eq!(read_secret(&c, "telegram_bot_token").as_deref(), Some("123456:FAKE-TOKEN"));
        assert!(std::fs::read_to_string(c.secrets_path()).unwrap().contains("sk-ant-OLD"));
        let json = serde_json::to_string(&st).unwrap();
        assert!(!json.contains("FAKE-TOKEN") && !json.contains("OLD"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(c.secrets_path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn old_settings_with_api_steps_still_load() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        std::fs::create_dir_all(c.space()).unwrap();
        std::fs::write(
            c.settings_path(),
            "model = \"claude-sonnet-5-5\"\n[setup]\ncompleted_steps = [\"folder\", \"api_key\", \"model\"]\ncompleted = true\n",
        )
        .unwrap();
        let s = settings_read(&c).unwrap().settings;
        assert_eq!(s.setup.completed_steps, ["folder", "api_key", "model"]);
        assert_eq!(s.claude_bin, "");
        assert!(valid_model(&s.model));
    }

    #[test]
    fn tilde_default_folder_is_stored_and_read_absolute() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path().join("home"));
        let mut s = settings_read(&c).unwrap().settings;
        s.default_folder = "~/".into();
        let w = settings_write(&c, SettingsWriteReq { settings: s }).unwrap();
        assert_eq!(Path::new(&w.settings.default_folder), c.home);
        assert!(!std::fs::read_to_string(c.settings_path()).unwrap().contains('~'));
        std::fs::write(c.settings_path(), "default_folder = \"~/x\"\n").unwrap();
        assert_eq!(Path::new(&settings_read(&c).unwrap().settings.default_folder), c.home.join("x"));
    }

    #[test]
    fn folder_check() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path().join("home"));
        let dir = d.path().join("work");
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(check_folder(&c, dir.to_str().unwrap()), json!({ "ok": true }));
        assert!(c.space().is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let file = d.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(check_folder(&c, file.to_str().unwrap())["ok"], false);
        assert_eq!(check_folder(&c, d.path().join("nope").to_str().unwrap())["ok"], false);
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn claude_check_reports_version_and_path() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let bin = script(d.path(), "claude", "echo '2.1.287 (Claude Code)'");
        let r = check_claude_at(&c, bin.to_str().unwrap());
        assert_eq!(r, json!({ "ok": true, "version": "2.1.287 (Claude Code)", "path": bin.to_str().unwrap() }));
        let mut s = settings_read(&c).unwrap().settings;
        s.claude_bin = bin.to_string_lossy().into_owned();
        settings_write(&c, SettingsWriteReq { settings: s }).unwrap();
        assert_eq!(check_claude(&c)["ok"], true);
        assert_eq!(check(&c, CheckReq { kind: "claude".into(), path: None }).unwrap()["version"], "2.1.287 (Claude Code)");
    }

    #[cfg(unix)]
    #[test]
    fn claude_check_failures_are_actionable() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let missing = check_claude_at(&c, d.path().join("nope").to_str().unwrap());
        assert_eq!(missing["ok"], false);
        assert!(missing["reason"].as_str().unwrap().contains("Nothing found"));
        let plain = d.path().join("plain");
        std::fs::write(&plain, "x").unwrap();
        let r = check_claude_at(&c, plain.to_str().unwrap());
        assert!(r["reason"].as_str().unwrap().contains("not an executable"));
        let bad = script(d.path(), "bad", "exit 3");
        assert_eq!(check_claude_at(&c, bad.to_str().unwrap())["ok"], false);
        let slow = script(d.path(), "slow", "exec sleep 5");
        let t = Instant::now();
        let r = check_claude_timeout(&c, slow.to_str().unwrap(), Duration::from_millis(200));
        assert!(t.elapsed() < Duration::from_secs(3));
        assert!(r["reason"].as_str().unwrap().contains("did not answer"));
    }

    #[test]
    fn execute_limits_default_clamp_and_validate() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let s = settings_read(&c).unwrap().settings;
        assert_eq!((s.max_budget_usd, s.execute_timeout_s), (0.0, 1800));
        std::fs::create_dir_all(c.space()).unwrap();
        std::fs::write(c.settings_path(), "max_budget_usd = 3\nexecute_timeout_s = 5\n").unwrap();
        let s = settings_read(&c).unwrap().settings;
        assert_eq!((s.max_budget_usd, s.execute_timeout_s), (3.0, 60));
        std::fs::write(c.settings_path(), "execute_timeout_s = 99999\nmax_budget_usd = -1.0\n").unwrap();
        let mut s = settings_read(&c).unwrap().settings;
        assert_eq!((s.max_budget_usd, s.execute_timeout_s), (0.0, 14400));
        s.max_budget_usd = -1.0;
        assert!(settings_write(&c, SettingsWriteReq { settings: s.clone() }).is_err());
        s.max_budget_usd = 1.0;
        s.execute_timeout_s = 10;
        assert!(settings_write(&c, SettingsWriteReq { settings: s }).is_err());
    }

    #[test]
    fn auth_parsing() {
        let max = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","subscriptionType":"max","email":"a@b.c"}"#;
        assert_eq!(parse_auth(max), Some((true, Some("claude.ai, max".into()))));
        assert_eq!(parse_auth(r#"{"loggedIn":true,"authMethod":"api_key"}"#), Some((true, Some("API key".into()))));
        assert_eq!(parse_auth(r#"{"loggedIn":true}"#), Some((true, None)));
        assert_eq!(parse_auth(r#"{"loggedIn":false}"#), Some((false, None)));
        assert_eq!(parse_auth("Not logged in"), None);
        assert_eq!(parse_auth(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn claude_check_reports_sign_in() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let body = |auth: &str| format!("if [ \"$1\" = auth ]; then echo '{auth}'; else echo '2.1.287 (Claude Code)'; fi");
        let ok = script(d.path(), "ok", &body(r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"a@b.c"}"#));
        let r = check_claude_at(&c, ok.to_str().unwrap());
        assert_eq!((r["ok"].clone(), r["logged_in"].clone(), r["auth"].clone()), (json!(true), json!(true), json!("claude.ai, max")));
        assert!(!r.to_string().contains("a@b.c"));
        let out = script(d.path(), "out", &body(r#"{"loggedIn":false}"#));
        let r = check_claude_at(&c, out.to_str().unwrap());
        assert_eq!((r["ok"].clone(), r["reason"].clone()), (json!(false), json!(NOT_SIGNED_IN)));
        let old = script(d.path(), "old", "if [ \"$1\" = auth ]; then echo 'unknown command' >&2; exit 1; fi; echo '1.0.0 (Claude Code)'");
        let r = check_claude_at(&c, old.to_str().unwrap());
        assert_eq!(r["ok"], true);
        assert!(r.get("logged_in").is_none() && r.get("auth").is_none());
    }

    #[test]
    fn claude_not_installed_message() {
        let d = tempfile::tempdir().unwrap();
        let c = Ctx::new(d.path());
        let r = check_claude_at(&c, "definitely-not-a-fabr-claude-binary");
        assert_eq!(r["ok"], false);
        assert!(r["reason"].as_str().unwrap().contains("PATH"));
        assert!(CLAUDE_MISSING.contains("https://claude.com/claude-code") && CLAUDE_MISSING.contains("press Check again"));
    }

    #[test]
    fn telegram_token_check() {
        let f = fake(vec![]);
        let (_d, c) = ctx(&f);
        assert_eq!(check_telegram_token(&c), json!({ "ok": true, "bot_name": "fabr_test_bot" }));
        assert!(f.log.lock().unwrap()[0].0.contains("bot123456:FAKE-TOKEN/getMe"));
        let d = tempfile::tempdir().unwrap();
        assert_eq!(check_telegram_token(&Ctx::new(d.path()))["ok"], false);
    }

    #[test]
    fn link_happy_path_ignores_other_users() {
        let f = fake(vec![msg(1, 7), msg(2, 99), tap(3, 99, "confirm"), tap(4, 7, "confirm")]);
        let (_d, c) = ctx(&f);
        assert_eq!(telegram_link(&c, 10), json!({ "ok": true, "user_id": 7 }));
        let log = f.log.lock().unwrap();
        let sends: Vec<_> = log.iter().filter(|(p, _)| p.ends_with("/sendMessage")).collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].1["body"]["text"], "connected");
        assert_eq!(sends[0].1["body"]["chat_id"], 7);
        assert_eq!(sends[0].1["body"]["reply_markup"]["inline_keyboard"][0][0]["text"], "Confirm");
        assert_eq!(sends[0].1["body"]["reply_markup"]["inline_keyboard"][0][0]["callback_data"], "confirm");
        let ans: Vec<_> = log.iter().filter(|(p, _)| p.ends_with("/answerCallbackQuery")).collect();
        assert_eq!(ans.len(), 1);
        assert_eq!(ans[0].1["body"]["callback_query_id"], "cb1");
    }

    #[test]
    fn link_times_out_without_a_message() {
        let f = fake(vec![]);
        let (_d, c) = ctx(&f);
        let r = telegram_link(&c, 1);
        assert_eq!(r, json!({ "ok": false, "reason": NO_MESSAGE }));
    }
}
