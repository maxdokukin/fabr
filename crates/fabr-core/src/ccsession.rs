use crate::error::FabrError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TITLE_CHARS: usize = 80;
const TEXT_CHARS: usize = 2000;
const TOOL_CHARS: usize = 160;
const PROMPT_CHARS: usize = 300;
const REPLY_CHARS: usize = 800;
const DIGEST_CHARS: usize = 3000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub modified_at: String,
    pub messages: usize,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<SessionTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTask {
    pub title: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackReq {
    pub folder: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackRes {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub title: String,
    pub entries: Vec<TranscriptEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionsReq {
    pub folder: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptReq {
    pub folder: String,
    pub id: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendReq {
    pub folder: String,
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendRes {
    pub reply: String,
    pub cost_usd: f64,
}

pub const SEND_TIMEOUT_S: u64 = 600;
const MAX_MESSAGE: usize = 8000;

pub fn config_dir() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".")).join(".claude"),
    }
}

pub fn encode_folder(folder: &Path) -> String {
    folder.to_string_lossy().trim_end_matches('/').chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub fn project_dir(config: &Path, folder: &Path) -> PathBuf {
    let abs = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    config.join("projects").join(encode_folder(&abs))
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 36 && id.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

pub fn list(config: &Path, folder: &Path) -> Result<Vec<SessionSummary>, FabrError> {
    let dir = project_dir(config, folder);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let linked = linked_tasks(folder);
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".jsonl").filter(|id| valid_id(id)) else { continue };
        let Ok(meta) = ent.metadata() else { continue };
        let text = std::fs::read_to_string(ent.path()).unwrap_or_default();
        let scan = scan_lines(&text);
        if scan.messages == 0 {
            continue;
        }
        let modified_at = meta
            .modified()
            .ok()
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default();
        out.push(SessionSummary {
            id: id.to_string(),
            title: scan.title,
            modified_at,
            messages: scan.messages,
            bytes: meta.len(),
            task: linked.get(id).cloned(),
        });
    }
    out.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.id.cmp(&b.id)));
    Ok(out)
}

pub fn transcript(config: &Path, folder: &Path, id: &str, limit: usize) -> Result<Transcript, FabrError> {
    if !valid_id(id) {
        return Err(FabrError::BadRequest(format!("{id} is not a session id")));
    }
    let path = project_dir(config, folder).join(format!("{id}.jsonl"));
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FabrError::NotFound(format!("no Claude Code session {id} for {}", folder.display())));
        }
        Err(e) => return Err(e.into()),
    };
    let scan = scan_lines(&text);
    let mut entries: Vec<TranscriptEntry> =
        text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).flat_map(|v| entries_of(&v)).collect();
    let base = format!("{}/", folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf()).to_string_lossy().trim_end_matches('/'));
    for e in entries.iter_mut().filter(|e| e.role == "tool") {
        if e.text.contains(&base) {
            e.text = e.text.replace(&base, "");
        }
    }
    let limit = limit.max(1);
    let truncated = entries.len() > limit;
    if truncated {
        entries.drain(..entries.len() - limit);
    }
    Ok(Transcript { id: id.to_string(), title: scan.title, entries, truncated })
}

pub fn resolve_bin(configured: &str) -> Option<PathBuf> {
    let env = std::env::var("FABR_CLAUDE_BIN").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let name = env.unwrap_or_else(|| configured.trim().to_string());
    let name = if name.is_empty() { "claude".to_string() } else { name };
    let name = match (name.strip_prefix("~/"), directories::BaseDirs::new()) {
        (Some(rest), Some(d)) => d.home_dir().join(rest).to_string_lossy().into_owned(),
        _ => name,
    };
    if name.contains('/') {
        let p = PathBuf::from(&name);
        return p.is_file().then_some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&name)).find(|p| p.is_file())
}

fn task_files(folder: &Path) -> Vec<(PathBuf, crate::taskrec::TaskRecord)> {
    let Ok(rd) = std::fs::read_dir(folder.join(".fabric-space").join("tasks")) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")).collect();
    paths.sort();
    paths.into_iter().filter_map(|p| Some((p.clone(), crate::taskrec::parse(&std::fs::read_to_string(&p).ok()?).ok()?))).collect()
}

fn roles(rec: &crate::taskrec::TaskRecord) -> [(&'static str, &Option<String>); 3] {
    [("plan", &rec.sessions.plan), ("execute", &rec.sessions.execute), ("evaluate", &rec.sessions.evaluate)]
}

const PICKED_UP: &str = "Picked up from Claude Code session ";

fn source_of(rec: &crate::taskrec::TaskRecord) -> Option<&str> {
    rec.context.lines().next()?.strip_prefix(PICKED_UP)?.strip_suffix('.').filter(|id| valid_id(id))
}

fn linked_tasks(folder: &Path) -> std::collections::HashMap<String, SessionTask> {
    let mut out = std::collections::HashMap::new();
    let files = task_files(folder);
    for (_, rec) in &files {
        for (role, id) in roles(rec) {
            if let Some(id) = id {
                out.entry(id.clone()).or_insert_with(|| SessionTask { title: rec.title.clone(), role: role.into() });
            }
        }
    }
    for (_, rec) in &files {
        if let Some(id) = source_of(rec) {
            out.entry(id.to_string()).or_insert_with(|| SessionTask { title: rec.title.clone(), role: "source".into() });
        }
    }
    out
}

fn owner(folder: &Path, id: &str) -> Option<(PathBuf, crate::taskrec::TaskRecord)> {
    task_files(folder).into_iter().find(|(_, rec)| roles(rec).iter().any(|(_, v)| v.as_deref() == Some(id)))
}

pub fn track(config: &Path, folder: &Path, id: &str) -> Result<String, FabrError> {
    let t = transcript(config, folder, id, usize::MAX)?;
    let marker = format!("{PICKED_UP}{id}");
    let space = folder.join(".fabric-space");
    std::fs::create_dir_all(&space)?;
    let lock = space.join("track.lock");
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock)?;
    held.lock().map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    if linked_tasks(folder).contains_key(id) {
        return Err(FabrError::Conflict { message: "This session is already tracked by a task.".into(), node: None });
    }
    let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let prompts: Vec<String> = t.entries.iter().filter(|e| e.role == "user").map(|e| clip(&flat(&e.text), PROMPT_CHARS)).collect();
    let title = crate::fabric::clean_title(&if t.title.is_empty() {
        prompts.first().map(|p| clip(p, TITLE_CHARS)).unwrap_or_default()
    } else {
        t.title.clone()
    });
    if title.is_empty() {
        return Err(FabrError::BadRequest("this session has no prompt to track".into()));
    }
    let reply = t.entries.iter().rev().find(|e| e.role == "assistant").map(|e| clip(&flat(&e.text), REPLY_CHARS));
    let mut ctx = format!("{marker}.");
    let mut budget = DIGEST_CHARS.saturating_sub(ctx.chars().count() + REPLY_CHARS + 80);
    let mut shown = 0;
    for p in &prompts {
        let line = format!("\n- {p}");
        let n = line.chars().count();
        if n > budget {
            break;
        }
        if shown == 0 {
            ctx.push_str("\n\nWhat was asked:");
        }
        budget -= n;
        ctx.push_str(&line);
        shown += 1;
    }
    if shown < prompts.len() {
        ctx.push_str(&format!("\n- ({} more)", prompts.len() - shown));
    }
    if let Some(r) = reply {
        ctx.push_str(&format!("\n\nLast reply:\n{r}"));
    }
    let abs = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    let req = crate::fabric::FabricCreateReq {
        folder: abs.to_string_lossy().into_owned(),
        title,
        due: None,
        pri: None,
        context: Some(ctx),
        spec: None,
        success: None,
    };
    Ok(crate::fabric::create_in(&req, &abs)?.path)
}

pub fn send(bin: &Path, folder: &Path, id: &str, message: &str, timeout: Duration) -> Result<SendRes, FabrError> {
    if !valid_id(id) {
        return Err(FabrError::BadRequest(format!("{id} is not a session id")));
    }
    let message = message.trim();
    if message.is_empty() || message.len() > MAX_MESSAGE {
        return Err(FabrError::BadRequest(format!("the message must be 1 to {MAX_MESSAGE} characters")));
    }
    let owned = owner(folder, id);
    if let Some((_, rec)) = &owned
        && rec.step.is_some()
        && !rec.waiting
        && !rec.paused
    {
        return Err(FabrError::Conflict {
            message: "This session is working on its task right now. Try again when the task waits or finishes.".into(),
            node: None,
        });
    }
    let marker = ChatMarker::take(folder, id)?;
    let mut cmd = Command::new(bin);
    cmd.args([
        "-p",
        "--resume",
        id,
        "--output-format",
        "json",
        "--permission-mode",
        "plan",
        "--permission-prompts",
        "none",
        "--tools",
        "Read,Glob,Grep",
    ])
    .current_dir(folder)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    own_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| FabrError::Io(format!("could not start Claude Code ({}): {e}", bin.display())))?;
    marker.hold(child.id());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(message.as_bytes());
    }
    let mut stdout = child.stdout.take().ok_or_else(|| FabrError::Io("no stdout".into()))?;
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            kill_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err(FabrError::Timeout(format!("Claude Code did not answer in {}s", timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = reader.join().unwrap_or_default();
    let v: Value = serde_json::from_str(out.trim()).map_err(|_| FabrError::Remote("Claude Code did not return a result".into()))?;
    let reply = v["result"].as_str().unwrap_or("").trim().to_string();
    if v["is_error"].as_bool().unwrap_or(false) {
        return Err(FabrError::Remote(format!("Claude Code reported an error: {}", clip(&reply, 300))));
    }
    let cost_usd = v["total_cost_usd"].as_f64().unwrap_or(0.0);
    if let Some((path, _)) = owned
        && cost_usd > 0.0
    {
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let _ = crate::taskrec::modify_record(&path, &now, |r| r.cost_usd = ((r.cost_usd + cost_usd) * 1e6).round() / 1e6);
    }
    Ok(SendRes { reply, cost_usd })
}

pub fn chat_marker(folder: &Path, id: &str) -> PathBuf {
    folder.join(".fabric-space").join("chat").join(format!("{id}.lock"))
}

struct ChatMarker(PathBuf);

impl ChatMarker {
    fn take(folder: &Path, id: &str) -> Result<ChatMarker, FabrError> {
        let path = chat_marker(folder, id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        for _ in 0..2 {
            let tmp = pid_file(&path, std::process::id())?;
            let linked = std::fs::hard_link(tmp.path(), &path).or_else(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => Err(e),
                _ => std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .and_then(|mut f| f.write_all(std::process::id().to_string().as_bytes())),
            });
            match linked {
                Ok(()) => return Ok(ChatMarker(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<u32>().ok());
                    if pid.map_or_else(|| marker_fresh(&path), pid_alive) {
                        return Err(FabrError::Conflict { message: "A chat with this session is already running.".into(), node: None });
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(FabrError::Conflict { message: "A chat with this session is already running.".into(), node: None })
    }

    fn hold(&self, pid: u32) {
        if let Ok(tmp) = pid_file(&self.0, pid) {
            let _ = tmp.persist(&self.0);
        }
    }
}

fn pid_file(marker: &Path, pid: u32) -> Result<tempfile::NamedTempFile, FabrError> {
    let mut tmp = tempfile::NamedTempFile::new_in(marker.parent().unwrap_or(Path::new(".")))?;
    tmp.write_all(pid.to_string().as_bytes())?;
    Ok(tmp)
}

pub fn marker_fresh(marker: &Path) -> bool {
    std::fs::metadata(marker).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age < Duration::from_secs(10)))
}

pub fn own_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id();
        // SAFETY: the hook runs between fork and exec and only calls prctl(2), getppid(2) and _exit(2), which are async-signal-safe.
        unsafe {
            cmd.pre_exec(move || {
                unsafe extern "C" {
                    fn prctl(option: i32, ...) -> i32;
                    fn getppid() -> i32;
                    fn _exit(code: i32) -> !;
                }
                prctl(1, 15 as std::ffi::c_ulong);
                if u32::try_from(getppid()).ok() != Some(parent) {
                    _exit(1);
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = cmd;
}

impl Drop for ChatMarker {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: signals the process group that the child leads; nothing is shared.
        unsafe {
            kill(-(pid as i32), 15);
        }
        std::thread::sleep(Duration::from_millis(500));
        // SAFETY: as above, for any member that ignored SIGTERM.
        unsafe {
            kill(-(pid as i32), 9);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

struct Scan {
    title: String,
    messages: usize,
}

fn scan_lines(text: &str) -> Scan {
    let mut custom = None;
    let mut ai_title = None;
    let mut first_prompt = None;
    let mut messages = 0;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("custom-title") => custom = v.get("customTitle").and_then(Value::as_str).map(str::to_string).or(custom),
            Some("ai-title") => ai_title = v.get("aiTitle").and_then(Value::as_str).map(str::to_string).or(ai_title),
            Some("summary") if ai_title.is_none() => ai_title = v.get("summary").and_then(Value::as_str).map(str::to_string),
            Some("user") | Some("assistant") if !sidechain(&v) => {
                messages += 1;
                if first_prompt.is_none() && v["type"] == "user" {
                    first_prompt = user_text(&v).filter(|t| !t.trim().is_empty());
                }
            }
            _ => {}
        }
    }
    let title = custom.or(ai_title).or(first_prompt).map(|t| clip(t.lines().next().unwrap_or("").trim(), TITLE_CHARS)).unwrap_or_default();
    Scan { title, messages }
}

fn sidechain(v: &Value) -> bool {
    v.get("isSidechain").and_then(Value::as_bool).unwrap_or(false)
}

fn user_text(v: &Value) -> Option<String> {
    match &v["message"]["content"] {
        Value::String(s) => Some(s.clone()),
        Value::Array(blocks) => {
            let parts: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

fn entries_of(v: &Value) -> Vec<TranscriptEntry> {
    if sidechain(v) {
        return Vec::new();
    }
    let at = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
    let entry = |role: &str, text: String, tool: Option<String>| TranscriptEntry { role: role.into(), text, tool, at: at.clone() };
    match v.get("type").and_then(Value::as_str) {
        Some("user") => match &v["message"]["content"] {
            Value::String(s) if !s.trim().is_empty() => vec![entry("user", clip(s, TEXT_CHARS), None)],
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| match b["type"].as_str() {
                    Some("text") => b["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| entry("user", clip(t, TEXT_CHARS), None)),
                    Some("tool_result") if b["is_error"].as_bool() == Some(true) => {
                        Some(entry("tool", clip(&result_text(&b["content"]), TOOL_CHARS), Some("error".into())))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        Some("assistant") => v["message"]["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| match b["type"].as_str() {
                        Some("text") => {
                            b["text"].as_str().filter(|t| !t.trim().is_empty()).map(|t| entry("assistant", clip(t, TEXT_CHARS), None))
                        }
                        Some("tool_use") if b["name"] == "StructuredOutput" => None,
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or("tool").to_string();
                            Some(entry("tool", tool_summary(&name, &b["input"]), Some(name)))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

pub fn tool_summary(name: &str, input: &Value) -> String {
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    let arg = match name {
        "Bash" => field("command"),
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => field("file_path").or_else(|| field("notebook_path")),
        "Glob" | "Grep" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Agent" | "Task" => field("description"),
        _ => None,
    };
    let arg = arg.unwrap_or_default().replace('\n', " ");
    clip(arg.trim(), TOOL_CHARS)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "2e6225e8-f0da-4b18-8eca-40b44d4bae82";

    fn fixture(home: &Path, folder: &Path) {
        let dir = project_dir(home, folder);
        std::fs::create_dir_all(&dir).unwrap();
        let inside = folder.canonicalize().unwrap().join("notes/b.md");
        let lines = [
            r#"{"type":"mode","mode":"normal"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":"Rename the scans\nby date"},"timestamp":"2026-10-30T10:00:00Z"}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Looking."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la\nscans"}}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"denied"}]}}"#.into(),
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"hidden"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/x/a.md"}},{"type":"tool_use","id":"t3","name":"StructuredOutput","input":{}},{"type":"text","text":"Done."}]}}"#.into(),
            format!(r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t4","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#, inside.display()),
            "not json".into(),
        ];
        std::fs::write(dir.join(format!("{ID}.jsonl")), lines.join("\n")).unwrap();
        std::fs::write(dir.join("notes.jsonl"), "{}").unwrap();
    }

    #[test]
    fn encoding_matches_claude_code() {
        assert_eq!(encode_folder(Path::new("/home/user/maxdokukin")), "-home-user-maxdokukin");
        assert_eq!(encode_folder(Path::new("/Users/a b/.x_y/")), "-Users-a-b--x-y");
        assert!(valid_id(ID) && !valid_id("../../etc/passwd") && !valid_id("2e6225e8f0da4b188eca40b44d4bae82"));
    }

    #[test]
    fn lists_and_reads_a_session_without_thinking_or_sidechains() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        assert!(list(home.path(), folder.path()).unwrap().is_empty());
        fixture(home.path(), folder.path());
        let all = list(home.path(), folder.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].id.as_str(), all[0].title.as_str(), all[0].messages), (ID, "Rename the scans", 5));
        let t = transcript(home.path(), folder.path(), ID, 100).unwrap();
        let roles: Vec<(&str, &str)> = t.entries.iter().map(|e| (e.role.as_str(), e.text.as_str())).collect();
        assert_eq!(
            roles,
            [
                ("user", "Rename the scans\nby date"),
                ("assistant", "Looking."),
                ("tool", "ls -la scans"),
                ("tool", "denied"),
                ("tool", "/x/a.md"),
                ("assistant", "Done."),
                ("tool", "notes/b.md")
            ]
        );
        assert_eq!(t.entries[2].tool.as_deref(), Some("Bash"));
        assert!(!t.truncated);
        let short = transcript(home.path(), folder.path(), ID, 2).unwrap();
        assert!(short.truncated && short.entries.len() == 2 && short.entries[0].text == "Done.");
        assert!(matches!(transcript(home.path(), folder.path(), "../x", 5), Err(FabrError::BadRequest(_))));
        let other = "00000000-0000-0000-0000-000000000000";
        assert!(matches!(transcript(home.path(), folder.path(), other, 5), Err(FabrError::NotFound(_))));
    }

    fn fake_bin(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("claude");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn send_resumes_read_only_and_refuses_a_busy_session() {
        let d = tempfile::tempdir().unwrap();
        let args = d.path().join("args");
        let bin = fake_bin(
            d.path(),
            &format!(
                "printf '%s\\n' \"$@\" > {}; cat > /dev/null; echo '{{\"result\":\"The plan reads notes.md first.\",\"total_cost_usd\":0.02,\"is_error\":false}}'",
                args.display()
            ),
        );
        let res = send(&bin, d.path(), ID, "Why read notes first?", Duration::from_secs(10)).unwrap();
        assert_eq!(res, SendRes { reply: "The plan reads notes.md first.".into(), cost_usd: 0.02 });
        let argv = std::fs::read_to_string(&args).unwrap();
        assert!(
            argv.contains(&format!("--resume\n{ID}\n"))
                && argv.contains("--permission-mode\nplan\n")
                && argv.contains("--tools\nRead,Glob,Grep\n")
        );
        assert!(matches!(send(&bin, d.path(), ID, "  ", Duration::from_secs(1)), Err(FabrError::BadRequest(_))));
        let tasks = d.path().join(".fabric-space/tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        let rec = format!(
            "---\nschema: 7\nid: t\ntitle: t\nstate: in_progress\nstep: execute\nrequests: {{delegate: false, cancel: false, redelegate: false}}\ngrants: {{}}\nlinks: {{}}\nsessions: {{plan: null, execute: {ID}, evaluate: null}}\n---\n## Context\n"
        );
        std::fs::write(tasks.join("t.md"), &rec).unwrap();
        assert!(matches!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)), Err(FabrError::Conflict { .. })));
        std::fs::write(tasks.join("t.md"), rec.replace("step: execute\n", "step: execute\nwaiting: true\n")).unwrap();
        assert!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)).is_ok());
        let after = crate::taskrec::parse(&std::fs::read_to_string(tasks.join("t.md")).unwrap()).unwrap();
        assert!((after.cost_usd - 0.02).abs() < 1e-9);
        std::fs::write(chat_marker(d.path(), ID), std::process::id().to_string()).unwrap();
        assert!(matches!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)), Err(FabrError::Conflict { .. })));
        std::fs::write(chat_marker(d.path(), ID), "999999999").unwrap();
        assert!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)).is_ok());
        assert!(!chat_marker(d.path(), ID).exists());
        let slow = fake_bin(d.path(), "sleep 5");
        assert!(matches!(send(&slow, d.path(), ID, "hi", Duration::from_millis(300)), Err(FabrError::Timeout(_))));
    }

    #[test]
    fn an_empty_marker_is_busy_while_fresh_and_stale_after_ten_seconds() {
        let d = tempfile::tempdir().unwrap();
        let bin = fake_bin(d.path(), "cat > /dev/null; echo '{\"result\":\"ok\",\"is_error\":false}'");
        let m = chat_marker(d.path(), ID);
        std::fs::create_dir_all(m.parent().unwrap()).unwrap();
        std::fs::write(&m, "").unwrap();
        assert!(matches!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)), Err(FabrError::Conflict { .. })));
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        std::fs::File::options().write(true).open(&m).unwrap().set_modified(old).unwrap();
        assert!(send(&bin, d.path(), ID, "hi", Duration::from_secs(10)).is_ok());
        assert!(!m.exists());
        let held = ChatMarker::take(d.path(), ID).unwrap();
        assert_eq!(std::fs::read_to_string(&m).unwrap(), std::process::id().to_string());
        held.hold(42);
        assert_eq!(std::fs::read_to_string(&m).unwrap(), "42");
        assert_eq!(std::fs::read_dir(m.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_in_its_own_group_runs_as_the_group_leader() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo $$ $(cut -d' ' -f5 /proc/$$/stat)"]).stdout(Stdio::piped());
        own_group(&mut cmd);
        let out = String::from_utf8(cmd.output().unwrap().stdout).unwrap();
        let ids: Vec<&str> = out.split_whitespace().collect();
        assert!(ids.len() == 2 && ids[0] == ids[1], "{out}");
    }

    #[test]
    fn an_ai_title_wins_over_the_first_prompt() {
        let text = format!("{}\n{}", r#"{"type":"user","message":{"content":"first"}}"#, r#"{"type":"ai-title","aiTitle":"Fix the CI"}"#);
        assert_eq!(scan_lines(&text).title, "Fix the CI");
        let named = format!("{text}\n{}", r#"{"type":"custom-title","customTitle":"fabr plan: x"}"#);
        assert_eq!(scan_lines(&named).title, "fabr plan: x");
        assert_eq!(clip("abcdef", 4), "abc…");
    }

    fn record(id: &str, extra: &str, ctx: &str) -> String {
        format!(
            "---\nschema: 7\nid: t\ntitle: Sort scans\nstate: in_progress\nstep: execute\nrequests: {{delegate: false, cancel: false, redelegate: false}}\ngrants: {{}}\nlinks: {{}}\nsessions: {{plan: {id}, execute: null, evaluate: null}}\n{extra}---\n## Context\n{ctx}"
        )
    }

    #[test]
    fn list_links_a_session_to_its_task() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fixture(home.path(), folder.path());
        assert_eq!(list(home.path(), folder.path()).unwrap()[0].task, None);
        let tasks = folder.path().join(".fabric-space/tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(tasks.join("t.md"), record(ID, "", "")).unwrap();
        let task = list(home.path(), folder.path()).unwrap()[0].task.clone();
        assert_eq!(task, Some(SessionTask { title: "Sort scans".into(), role: "plan".into() }));
    }

    #[test]
    fn track_makes_a_bounded_reminder_and_refuses_a_second_time() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fixture(home.path(), folder.path());
        let path = track(home.path(), folder.path(), ID).unwrap();
        let rec = crate::taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((rec.title.as_str(), rec.state), ("Rename the scans", crate::taskrec::State::Reminder));
        assert!(rec.sessions.plan.is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(&format!("Picked up from Claude Code session {ID}.")));
        assert!(text.contains("- Rename the scans by date") && text.contains("Last reply:\nDone."));
        assert!(matches!(track(home.path(), folder.path(), ID), Err(FabrError::Conflict { .. })));
        assert!(matches!(track(home.path(), folder.path(), "../x"), Err(FabrError::BadRequest(_))));
    }

    #[test]
    fn a_tracked_session_shows_as_linked_by_its_first_context_line() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        fixture(home.path(), folder.path());
        track(home.path(), folder.path(), ID).unwrap();
        let task = list(home.path(), folder.path()).unwrap()[0].task.clone();
        assert_eq!(task, Some(SessionTask { title: "Rename the scans".into(), role: "source".into() }));
        let tasks = folder.path().join(".fabric-space/tasks");
        for e in std::fs::read_dir(&tasks).unwrap().flatten() {
            std::fs::remove_file(e.path()).unwrap();
        }
        std::fs::write(tasks.join("m.md"), record("other", "", &format!("Notes\nPicked up from Claude Code session {ID}.\n"))).unwrap();
        assert_eq!(list(home.path(), folder.path()).unwrap()[0].task, None);
    }

    #[test]
    fn control_characters_in_a_title_never_corrupt_the_record() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let dir = project_dir(home.path(), folder.path());
        std::fs::create_dir_all(&dir).unwrap();
        let line = r#"{"type":"custom-title","customTitle":"Fix\u001b[31m red\ttab\u0085x"}"#;
        std::fs::write(dir.join(format!("{ID}.jsonl")), format!("{}\n{line}", r#"{"type":"user","message":{"content":"hi"}}"#)).unwrap();
        let path = track(home.path(), folder.path(), ID).unwrap();
        let rec = crate::taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(rec.title, "Fix[31m red tabx");
    }

    #[test]
    fn track_clips_long_prompts_and_keeps_the_digest_small() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let dir = project_dir(home.path(), folder.path());
        std::fs::create_dir_all(&dir).unwrap();
        let mut lines: Vec<String> =
            (0..40).map(|i| format!(r#"{{"type":"user","message":{{"content":"{i} {}"}}}}"#, "word ".repeat(100))).collect();
        lines.push(format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#, "z".repeat(1500)));
        std::fs::write(dir.join(format!("{ID}.jsonl")), lines.join("\n")).unwrap();
        let path = track(home.path(), folder.path(), ID).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let ctx = text.split("## Context\n").nth(1).unwrap().split("## Spec").next().unwrap();
        assert!(ctx.chars().count() <= 3000, "{}", ctx.chars().count());
        assert!(ctx.contains("more)") && ctx.contains(&format!("{}…", "z".repeat(799))));
        assert!(ctx.lines().filter(|l| l.starts_with("- ")).all(|l| l.chars().count() <= 303));
    }
}
