use crate::caps::{on_path, runnable};
use crate::engine::{
    DefineInput, DefineOut, Denial, End, Engine, EvaluateInput, EvaluateOut, ExecStatus, ExecuteInput, ExecuteOut, PlanInput, PlanOut, Run,
    ToolUse, Usage, Watch,
};
use fabr_core::FabrError;
use fabr_core::taskrec::Assertion;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT_S: u64 = 1800;
pub const READ_ONLY_TOOLS: &str = "Read,Glob,Grep";
const POLL: Duration = Duration::from_secs(1);
const STDERR_CAP: usize = 8 * 1024;

const PLANNER: &str = "You are the planner for fabr, an app that turns the user's reminders into tasks that Claude Code carries out. \
The current directory is the task's folder. You only read: never edit files and never run commands. \
Answer only through the structured output.";
const EXECUTOR: &str = "You are the executor for a task delegated from fabr. The current directory is the task's folder. \
Create and change files only inside this folder; never touch anything outside it. Follow the approved plan. \
If something required is missing, or you need a tool or command you were not given, do not work around it: \
stop and return status ask with one question for the user, or status blocked. \
When the work is finished, return status done with a short summary of what changed.";
const VALIDATOR: &str = "You are the validator for a task delegated from fabr. The current directory is the task's folder. \
Judge with fresh eyes whether the finished work meets each assertion. Read the files to check; never change anything. \
Answer only through the structured output.";

#[derive(Debug, Clone)]
pub struct ClaudeEngine {
    pub bin: PathBuf,
    pub model: String,
    pub effort: String,
    pub timeout: Duration,
    pub max_budget_usd: f64,
    pub runs_file: Option<PathBuf>,
}

pub const SPEND_CAP: &str = "spend cap reached";

impl ClaudeEngine {
    pub fn new(bin: impl Into<PathBuf>) -> ClaudeEngine {
        ClaudeEngine {
            bin: bin.into(),
            model: String::new(),
            effort: String::new(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_S),
            max_budget_usd: 0.0,
            runs_file: None,
        }
    }

    pub fn with_model(mut self, model: &str, effort: &str) -> ClaudeEngine {
        self.model = model.trim().to_string();
        self.effort = effort.trim().to_string();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> ClaudeEngine {
        self.timeout = timeout;
        self
    }

    pub fn with_budget(mut self, usd: f64) -> ClaudeEngine {
        self.max_budget_usd = if usd.is_finite() && usd > 0.0 { usd } else { 0.0 };
        self
    }

    pub fn with_runs_file(mut self, file: impl Into<PathBuf>) -> ClaudeEngine {
        self.runs_file = Some(file.into());
        self
    }
}

pub const STATE_DIR: &str = ".fabric-space";

pub fn state_deny_rules() -> Vec<String> {
    let mut out = Vec::new();
    for tool in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
        for dir in ["./.fabric-space/**", ".fabric-space/**"] {
            out.push(format!("{tool}({dir})"));
        }
    }
    out
}

pub fn is_session_id(id: &str) -> bool {
    id.len() == 36 && id.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Planner,
    Executor { read_only: bool },
    Validator,
}

pub struct Call<'a> {
    pub role: Role,
    pub cwd: &'a Path,
    pub resume: Option<&'a str>,
    pub prompt: String,
    pub schema: Value,
    pub allowed: Vec<String>,
    pub name: String,
}

pub fn session_name(role: Role, title: &str) -> String {
    let word = match role {
        Role::Planner => "plan",
        Role::Executor { .. } => "execute",
        Role::Validator => "validate",
    };
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
    format!("fabr {word}: {title}")
}

pub fn resolve_bin(configured: &str) -> Option<PathBuf> {
    let env = std::env::var("FABR_CLAUDE_BIN").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let name = env.unwrap_or_else(|| configured.trim().to_string());
    let name = if name.is_empty() { "claude".to_string() } else { name };
    let name = match (name.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest).to_string_lossy().into_owned(),
        _ => name,
    };
    if name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(&name);
        runnable(&path).then_some(path)
    } else {
        on_path(&name)
    }
}

pub fn new_session_id() -> String {
    let mut b = rand::random::<u128>().to_be_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

pub fn args(engine: &ClaudeEngine, call: &Call, session: &str) -> Vec<String> {
    let mut out: Vec<String> = vec!["-p".into(), "--output-format".into(), "stream-json".into(), "--verbose".into()];
    out.push("--json-schema".into());
    out.push(call.schema.to_string());
    match call.resume.filter(|id| is_session_id(id)) {
        Some(id) => out.extend(["--resume".into(), id.to_string()]),
        None => out.extend(["--session-id".into(), session.to_string()]),
    }
    let (mode, tools, system) = match call.role {
        Role::Planner => ("plan", Some(READ_ONLY_TOOLS), PLANNER),
        Role::Validator => ("plan", Some(READ_ONLY_TOOLS), VALIDATOR),
        Role::Executor { read_only } => ("acceptEdits", read_only.then_some(READ_ONLY_TOOLS), EXECUTOR),
    };
    out.extend(["--permission-mode".into(), mode.into(), "--permission-prompts".into(), "none".into()]);
    if let Some(tools) = tools {
        out.extend(["--tools".into(), tools.into()]);
    }
    let allowed = if call.allowed.is_empty() { READ_ONLY_TOOLS.replace(',', " ") } else { call.allowed.join(" ") };
    out.extend(["--allowedTools".into(), allowed]);
    if matches!(call.role, Role::Executor { .. }) {
        out.extend(["--disallowedTools".into(), state_deny_rules().join(" ")]);
    }
    if !engine.model.is_empty() {
        out.extend(["--model".into(), engine.model.clone()]);
    }
    if !engine.effort.is_empty() {
        out.extend(["--effort".into(), engine.effort.clone()]);
    }
    if engine.max_budget_usd > 0.0 {
        out.extend(["--max-budget-usd".into(), format!("{:.2}", engine.max_budget_usd)]);
    }
    if !call.name.is_empty() {
        out.extend(["--name".into(), call.name.clone()]);
    }
    out.extend(["--append-system-prompt".into(), system.into()]);
    out
}

pub fn session<T: DeserializeOwned>(engine: &ClaudeEngine, call: &Call, watch: &mut dyn Watch) -> Run<T> {
    if ABORT.load(Ordering::SeqCst) {
        return Run {
            session_id: call.resume.map(str::to_string),
            cost_usd: 0.0,
            usage: Usage::default(),
            denials: Vec::new(),
            end: End::Stopped,
        };
    }
    let fresh = new_session_id();
    let session = call.resume.filter(|id| is_session_id(id)).unwrap_or(&fresh).to_string();
    let mut cmd = Command::new(&engine.bin);
    cmd.args(args(engine, call, &session)).current_dir(call.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::pgroup::own_group(&mut cmd);
    let mut tries = 0;
    let mut child = loop {
        match cmd.spawn() {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Run::failed(FabrError::Io(format!("could not start Claude Code ({}): {e}", engine.bin.display()))),
        }
    };
    let _live = Live::add(child.id(), engine.runs_file.clone());
    if let Some(mut stdin) = child.stdin.take() {
        let prompt = call.prompt.clone();
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
    }
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let stderr = child.stderr.take().map(|s| std::thread::spawn(move || capped(s)));
    let mut run: Run<T> = Run { session_id: None, cost_usd: 0.0, usage: Usage::default(), denials: Vec::new(), end: End::Stopped };
    let mut result: Option<Value> = None;
    let mut turns: Vec<(String, u64, u64)> = Vec::new();
    let deadline = Instant::now() + engine.timeout;
    let mut polled = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => on_line(&line, &mut run, &mut result, &mut turns, watch),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if ABORT.load(Ordering::SeqCst) {
            kill(&mut child);
            return run;
        }
        if polled.elapsed() >= POLL {
            polled = Instant::now();
            if watch.stop() {
                kill(&mut child);
                return run;
            }
        }
        if Instant::now() >= deadline {
            kill(&mut child);
            let u = &run.usage;
            run.end = End::Failed(FabrError::Timeout(format!(
                "Claude Code ran longer than {}s and was stopped after {} turns ({} input and {} output tokens; cost unknown)",
                engine.timeout.as_secs(),
                u.turns,
                u.input_tokens,
                u.output_tokens
            )));
            return run;
        }
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                kill(&mut child);
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    if ABORT.load(Ordering::SeqCst) {
        kill(&mut child);
        return run;
    }
    let err_text = stderr.and_then(|h| h.join().ok()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let flat = err_text.split_whitespace().collect::<Vec<_>>().join(" ");
    let err_tail: String = flat.chars().skip(flat.chars().count().saturating_sub(300)).collect();
    run.end = finish(status.and_then(|s| s.code()), result, &err_tail, &mut run);
    run
}

fn on_line<T>(line: &str, run: &mut Run<T>, result: &mut Option<Value>, turns: &mut Vec<(String, u64, u64)>, watch: &mut dyn Watch) {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return };
    match v["type"].as_str() {
        Some("system") if v["subtype"].as_str() == Some("init") => {
            if let Some(id) = v["session_id"].as_str() {
                run.session_id = Some(id.to_string());
                watch.session(id);
            }
        }
        Some("assistant") => {
            tally(&v["message"], turns, &mut run.usage);
            for block in v["message"]["content"].as_array().map(Vec::as_slice).unwrap_or_default() {
                if block["type"].as_str() != Some("tool_use") {
                    continue;
                }
                let name = block["name"].as_str().unwrap_or("").to_string();
                if name.is_empty() || name == "StructuredOutput" {
                    continue;
                }
                watch.tool(&ToolUse { name, input: block["input"].clone() });
            }
        }
        Some("result") => *result = Some(v),
        _ => {}
    }
}

fn tally(message: &Value, turns: &mut Vec<(String, u64, u64)>, usage: &mut Usage) {
    let u = &message["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    let input = n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
    let output = n("output_tokens");
    let id = message["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("#{}", turns.len()));
    match turns.iter_mut().find(|t| t.0 == id) {
        Some(t) => (t.1, t.2) = (t.1.max(input), t.2.max(output)),
        None => turns.push((id, input, output)),
    }
    *usage =
        Usage { turns: turns.len() as u64, input_tokens: turns.iter().map(|t| t.1).sum(), output_tokens: turns.iter().map(|t| t.2).sum() };
}

fn finish<T: DeserializeOwned>(code: Option<i32>, result: Option<Value>, err_tail: &str, run: &mut Run<T>) -> End<T> {
    let Some(r) = result else {
        let code = code.map_or("no exit code".to_string(), |c| format!("exit {c}"));
        let detail = if err_tail.is_empty() { String::new() } else { format!(": {err_tail}") };
        return End::Failed(FabrError::Remote(format!("Claude Code ended without a result ({code}){detail}")));
    };
    run.cost_usd = r["total_cost_usd"].as_f64().unwrap_or(0.0);
    let n = |k: &str| r["usage"][k].as_u64().unwrap_or(0);
    run.usage = Usage {
        turns: r["num_turns"].as_u64().unwrap_or(0),
        input_tokens: n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens"),
        output_tokens: n("output_tokens"),
    };
    if let Some(id) = r["session_id"].as_str() {
        run.session_id = Some(id.to_string());
    }
    run.denials = r["permission_denials"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|d| Denial { tool: d["tool_name"].as_str().unwrap_or("").to_string(), input: d["tool_input"].clone() })
                .filter(|d| !d.tool.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let subtype = r["subtype"].as_str().unwrap_or("unknown");
    if subtype.contains("budget") {
        return End::Failed(FabrError::Remote(format!("{SPEND_CAP}: Claude Code stopped after ${:.2}", run.cost_usd)));
    }
    if r["is_error"].as_bool().unwrap_or(false) || subtype != "success" {
        let text = r["result"].as_str().unwrap_or("").trim();
        let detail = if text.is_empty() { err_tail } else { text };
        return End::Failed(FabrError::Remote(format!("Claude Code reported {subtype}: {detail}")));
    }
    if let Some(c) = code.filter(|c| *c != 0) {
        return End::Failed(FabrError::Remote(format!("Claude Code exited with {c}: {err_tail}")));
    }
    match r.get("structured_output").filter(|v| !v.is_null()).cloned().or_else(|| json_in(r["result"].as_str().unwrap_or(""))) {
        None => End::Failed(FabrError::Unprocessable(format!("{NO_STRUCTURED}: {}", r["result"].as_str().unwrap_or("").trim()))),
        Some(v) => match serde_json::from_value::<T>(v) {
            Ok(out) => End::Done(out),
            Err(e) => End::Failed(FabrError::Remote(format!("Claude Code structured output did not match: {e}"))),
        },
    }
}

const NO_STRUCTURED: &str = "Claude Code returned no structured output";
const NUDGE: &str = "Your work for this step is finished. Return the final result now by calling the StructuredOutput tool with the required fields. Do nothing else.";

fn json_in(text: &str) -> Option<Value> {
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    (start < end).then(|| serde_json::from_str::<Value>(&text[start..=end]).ok()).flatten().filter(Value::is_object)
}

pub fn missing_text<T>(run: &Run<T>) -> Option<String> {
    match &run.end {
        End::Failed(FabrError::Unprocessable(m)) => m.strip_prefix(NO_STRUCTURED).map(|t| t.trim_start_matches(':').trim().to_string()),
        _ => None,
    }
}

pub fn strict<T: DeserializeOwned>(engine: &ClaudeEngine, call: &Call, watch: &mut dyn Watch) -> Run<T> {
    let first = session::<T>(engine, call, watch);
    let (Some(_), Some(id)) = (missing_text(&first), first.session_id.clone()) else { return first };
    let nudge = Call {
        role: call.role,
        cwd: call.cwd,
        resume: Some(&id),
        prompt: NUDGE.into(),
        schema: call.schema.clone(),
        allowed: call.allowed.clone(),
        name: call.name.clone(),
    };
    let mut second = session::<T>(engine, &nudge, watch);
    second.cost_usd += first.cost_usd;
    second.usage.turns += first.usage.turns;
    second.usage.input_tokens += first.usage.input_tokens;
    second.usage.output_tokens += first.usage.output_tokens;
    if second.session_id.is_none() {
        second.session_id = Some(id);
    }
    if missing_text(&second).is_some() {
        second.end = first.end;
    }
    second
}

type Tracked = (u32, Option<PathBuf>);

static LIVE: Mutex<Vec<Tracked>> = Mutex::new(Vec::new());
static ABORT: AtomicBool = AtomicBool::new(false);

struct Live(u32, Option<PathBuf>);

impl Live {
    fn add(pid: u32, file: Option<PathBuf>) -> Live {
        let mut live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
        live.push((pid, file.clone()));
        if let Some(f) = &file {
            write_runs(f, &live);
        }
        Live(pid, file)
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let mut live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
        live.retain(|p| p.0 != self.0);
        if let Some(f) = &self.1 {
            write_runs(f, &live);
        }
    }
}

fn run_lines(file: &Path) -> Vec<(u32, u32)> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace().map(|w| w.parse::<u32>().ok());
            Some((p.next()??, p.next()??))
        })
        .collect()
}

fn save_runs(file: &Path, lines: &[(u32, u32)]) {
    let text: String = lines.iter().map(|(g, o)| format!("{g} {o}\n")).collect();
    let res = if text.is_empty() {
        std::fs::remove_file(file).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
    } else {
        crate::runner::write_atomic(file, &text)
    };
    if let Err(e) = res {
        log::warn!("could not update {}: {e}", file.display());
    }
}

fn write_runs(file: &Path, live: &[Tracked]) {
    let me = std::process::id();
    let mut lines: Vec<(u32, u32)> = run_lines(file).into_iter().filter(|(_, o)| *o != me && crate::pgroup::alive(*o)).collect();
    lines.extend(live.iter().filter(|t| t.1.as_deref() == Some(file)).map(|t| (t.0, me)));
    save_runs(file, &lines);
}

pub fn reap_orphans(file: &Path) -> Vec<String> {
    let me = std::process::id();
    let mut kept = Vec::new();
    let mut out = Vec::new();
    for (group, owner) in run_lines(file) {
        if owner == me || crate::pgroup::alive(owner) {
            kept.push((group, owner));
            continue;
        }
        if !crate::pgroup::group_alive(group) {
            continue;
        }
        crate::pgroup::signal_group(group, crate::pgroup::TERM);
        let until = Instant::now() + Duration::from_secs(2);
        while crate::pgroup::group_alive(group) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        crate::pgroup::signal_group(group, crate::pgroup::KILL);
        out.push(format!("killed leftover Claude Code process group {group} from runner {owner}"));
    }
    save_runs(file, &kept);
    out
}

pub fn live_runs() -> usize {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner).len()
}

pub fn abort_all() {
    ABORT.store(true, Ordering::SeqCst);
    for (pid, _) in LIVE.lock().unwrap_or_else(PoisonError::into_inner).iter() {
        crate::pgroup::signal_group(*pid, crate::pgroup::TERM);
    }
}

pub fn allow_runs() {
    ABORT.store(false, Ordering::SeqCst);
}

fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        crate::pgroup::signal_group(child.id(), crate::pgroup::TERM);
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        crate::pgroup::signal_group(child.id(), crate::pgroup::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn capped(mut reader: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > STDERR_CAP {
                    kept.drain(..kept.len() - STDERR_CAP);
                }
            }
        }
    }
    kept
}

fn assertion_lines(assertions: &[Assertion]) -> String {
    assertions.iter().map(|a| format!("{}: {}", a.id, a.text)).collect::<Vec<_>>().join("\n")
}

fn assertions_schema() -> Value {
    json!({"type": "array", "maxItems": 3, "items": {"type": "object", "properties": {"id": {"type": "string"}, "text": {"type": "string"}}, "required": ["id", "text"]}})
}

pub fn define_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "question": {"type": ["string", "null"]},
            "spec": {"type": "string"},
            "success": {"type": "string"},
            "assertions": assertions_schema(),
            "grants": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["question", "spec", "success", "assertions"]
    })
}

pub fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "plan": {"type": "string"},
            "reversibility": {"type": "string", "enum": ["reversible", "irreversible"]},
            "tools": {"type": "array", "items": {"type": "string"}},
            "checks": {"type": "array", "items": {"type": "object", "properties": {"cmd": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}}}, "required": ["cmd", "args"]}}
        },
        "required": ["plan", "reversibility", "tools", "checks"]
    })
}

pub fn execute_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "enum": ["done", "ask", "blocked"]},
            "summary": {"type": "string"},
            "question": {"type": ["string", "null"]}
        },
        "required": ["status", "summary", "question"]
    })
}

pub fn evaluate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {"type": "array", "items": {"type": "object", "properties": {
                "id": {"type": "string"},
                "verdict": {"type": "string", "enum": ["pass", "fail", "unchecked"]},
                "evidence": {"type": "string"}
            }, "required": ["id", "verdict", "evidence"]}}
        },
        "required": ["items"]
    })
}

const DEFINE_ASK: &str = "Return:\n\
- question: one short question for the user when something required is missing and cannot be found by reading the folder, otherwise null. Ask at most one.\n\
- spec: what will be done, in a few plain sentences (empty when you ask).\n\
- success: how the user will know it is done (empty when you ask).\n\
- assertions: 1 to 3 checkable statements about the finished result, ids a1, a2, a3, as few as the success definition needs. \
Each one must be checkable from the files in this folder; no filler.\n\
Read files in the folder when it helps.";

pub fn define_prompt(input: &DefineInput) -> String {
    if input.resume.is_some() {
        return format!(
            "The user replied.{}\n\nDefine the task again with this in mind. Keep question null unless something required is still missing.\n\n{DEFINE_ASK}",
            input.answers
        );
    }
    format!(
        "Task title: {}\n\nContext (the user's notes and what fabr gathered from the folder):\n{}{}\n\nDefine this task.\n\n{DEFINE_ASK}",
        input.title, input.context, input.answers
    )
}

pub fn plan_prompt(input: &PlanInput) -> String {
    let mut out = String::new();
    if input.resume.is_none() {
        out.push_str(&format!("Task title: {}\n\nContext:\n{}\n\n", input.title, input.context));
    }
    out.push_str(&format!(
        "Plan the task.\n\nSpec:\n{}\n\nSuccess:\n{}\n\nAssertions:\n{}\n",
        input.spec,
        input.success,
        assertion_lines(input.assertions)
    ));
    if !input.answers.trim().is_empty() {
        out.push_str(&format!("\nThe user's answers so far:{}\n", input.answers));
    }
    if let Some(note) = input.refusal_note.filter(|n| !n.trim().is_empty()) {
        out.push_str(&format!("\nThe user refused the previous plan with this note: {note}\n"));
    } else if input.refusal_note.is_some() {
        out.push_str("\nThe user refused the previous plan; propose a different one.\n");
    }
    if let Some(r) = input.rejection {
        out.push_str(&format!("\nYour previous plan was rejected: {r}. Return a corrected plan.\n"));
    }
    out.push_str(&format!(
        "\nReturn:\n\
         - plan: markdown with exactly three subsections, in this order: `### Inputs` (the files, folders and facts the work starts from), \
         `### Outputs` (every file that will be created or changed, all inside this folder), `### Steps` (numbered steps).\n\
         - reversibility: reversible, or irreversible when a step cannot be undone (sending, paying, deleting without a copy).\n\
         - tools: the Claude Code permission rules the executor needs beyond Read, Glob and Grep, for example Edit, Write or Bash(npm test:*). \
         Choose only from these tools: {}. Ask for as little as possible; Bash always needs a command pattern in parentheses.\n\
         - checks: commands that prove the result after the work is done, as objects with cmd and args, run without a shell in this folder. \
         cmd must be one of: {}. Every file a check names must exist now or be created by the plan.",
        input.allowed_tools.join(", "),
        input.allowed_checks.join(", ")
    ));
    out
}

pub fn execute_prompt(input: &ExecuteInput) -> String {
    if input.resume.is_some() {
        let mut out = String::from("Continue the task from where you stopped.");
        if !input.answers.trim().is_empty() {
            out.push_str(&format!("\n\nThe user's answers so far:{}", input.answers));
        }
        out.push_str(&format!("\n\nYou may now use: {}.", input.allowed.join(", ")));
        out.push_str("\n\nWhen you are finished or stuck, return the structured result.");
        return out;
    }
    let mut out = format!(
        "Task: {}\n\nSpec:\n{}\n\nApproved plan:\n{}\n\nAssertions that will be checked afterwards:\n{}\n\nYou may use: {}.",
        input.title,
        input.spec,
        input.plan,
        assertion_lines(input.assertions),
        input.allowed.join(", ")
    );
    if input.read_only {
        out.push_str(" This machine runs tasks read-only: do not change any file.");
    }
    if !input.answers.trim().is_empty() {
        out.push_str(&format!("\n\nThe user's answers so far:{}", input.answers));
    }
    out.push_str("\n\nCarry out the plan now, then return the structured result.");
    out
}

pub fn evaluate_prompt(input: &EvaluateInput) -> String {
    let checks: Vec<String> =
        input.checks.iter().map(|c| format!("{} [{}]", c.evidence, if c.pass { "exit 0" } else { "non-zero" })).collect();
    format!(
        "Task: {}\n\nAssertions:\n{}\n\nThe executor's summary:\n{}\n\nExecute log (every tool call the executor made):\n{}\n\nCheck results:\n{}\n\n\
         Return items with one entry per assertion id. verdict is pass or fail, or unchecked only when nothing available can settle it. \
         evidence is one line. A check that names a missing file is evidence of absence, not an automatic failure.",
        input.title,
        assertion_lines(input.assertions),
        if input.summary.trim().is_empty() { "(none)" } else { input.summary },
        if input.log.trim().is_empty() { "(none)" } else { input.log },
        if checks.is_empty() { "(none)".to_string() } else { checks.join("\n") }
    )
}

impl Engine for ClaudeEngine {
    fn define(&self, input: &DefineInput, watch: &mut dyn Watch) -> Run<DefineOut> {
        let call = Call {
            role: Role::Planner,
            cwd: input.cwd,
            resume: input.resume,
            prompt: define_prompt(input),
            schema: define_schema(),
            allowed: vec![],
            name: session_name(Role::Planner, input.title),
        };
        strict(self, &call, watch)
    }

    fn plan(&self, input: &PlanInput, watch: &mut dyn Watch) -> Run<PlanOut> {
        let call = Call {
            role: Role::Planner,
            cwd: input.cwd,
            resume: input.resume,
            prompt: plan_prompt(input),
            schema: plan_schema(),
            allowed: vec![],
            name: session_name(Role::Planner, input.title),
        };
        strict(self, &call, watch)
    }

    fn execute(&self, input: &ExecuteInput, watch: &mut dyn Watch) -> Run<ExecuteOut> {
        let allowed = if input.read_only {
            input.allowed.iter().filter(|r| crate::caps::ALWAYS_GRANTED.contains(&crate::caps::rule_tool(r))).cloned().collect()
        } else {
            input.allowed.to_vec()
        };
        let call = Call {
            role: Role::Executor { read_only: input.read_only },
            cwd: input.cwd,
            resume: input.resume,
            prompt: execute_prompt(input),
            schema: execute_schema(),
            allowed,
            name: session_name(Role::Executor { read_only: input.read_only }, input.title),
        };
        let mut run = strict::<ExecuteOut>(self, &call, watch);
        if let Some(text) = missing_text(&run).filter(|_| run.denials.is_empty()) {
            let summary = if text.is_empty() { "finished without a summary".to_string() } else { text.chars().take(2000).collect() };
            run.end = End::Done(ExecuteOut { status: ExecStatus::Done, summary, question: None });
        }
        run
    }

    fn evaluate(&self, input: &EvaluateInput, watch: &mut dyn Watch) -> Run<EvaluateOut> {
        let call = Call {
            role: Role::Validator,
            cwd: input.cwd,
            resume: None,
            prompt: evaluate_prompt(input),
            schema: evaluate_schema(),
            allowed: vec![],
            name: session_name(Role::Validator, input.title),
        };
        strict(self, &call, watch)
    }
}

pub fn tool_line(cwd: &Path, tool: &ToolUse) -> String {
    let s = |k: &str| tool.input[k].as_str().unwrap_or("").to_string();
    let arg = match tool.name.as_str() {
        "Read" | "Edit" | "Write" | "MultiEdit" => short_path(cwd, &s("file_path")),
        "NotebookEdit" => short_path(cwd, &s("notebook_path")),
        "Bash" => s("command").lines().next().unwrap_or("").to_string(),
        "Glob" | "Grep" => s("pattern"),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        _ => String::new(),
    };
    let arg: String = arg.split_whitespace().collect::<Vec<_>>().join(" ");
    let arg = if arg.chars().count() > 80 { format!("{}...", arg.chars().take(80).collect::<String>()) } else { arg };
    format!("{} {arg}", tool.name).trim_end().to_string()
}

fn short_path(cwd: &Path, path: &str) -> String {
    let p = Path::new(path);
    let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    p.strip_prefix(cwd).or_else(|_| p.strip_prefix(&canon)).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string())
}

pub fn inside(cwd: &Path, path: &str) -> bool {
    let p = Path::new(path);
    if p.is_relative() {
        return !p.components().any(|c| matches!(c, std::path::Component::ParentDir));
    }
    let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    (p.starts_with(cwd) || p.starts_with(&canon)) && !p.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

const WRAPPERS: [&str; 22] = [
    "env", "sudo", "su", "doas", "xargs", "eval", "exec", "command", "builtin", "nohup", "nice", "timeout", "time", "watch", "source", ".",
    "stdbuf", "setsid", "chroot", "flock", "ssh", "busybox",
];
const SHELLS: [&str; 8] = ["sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh"];
const INTERPRETERS: [&str; 10] = ["python", "node", "perl", "ruby", "deno", "bun", "php", "lua", "osascript", "pwsh"];

pub fn grantable_command(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some(first) = words.first() else { return false };
    let name = first.rsplit('/').next().unwrap_or(first);
    let inline = |w: &&str| {
        matches!(*w, "--eval" | "--print" | "--command" | "-p" | "-exec" | "-execdir" | "-ok")
            || (w.starts_with('-') && !w.starts_with("--") && w[1..].contains(['c', 'e', 'E']))
    };
    !command.contains(['\n', '\r', ';', '&', '|', '`', '*', '(', ')', '<', '>'])
        && !command.contains("$(")
        && !command.contains(STATE_DIR)
        && !first.contains('=')
        && !WRAPPERS.contains(&name)
        && !SHELLS.contains(&name)
        && !(INTERPRETERS.iter().any(|i| name.starts_with(i)) && words[1..].iter().any(inline))
        && !words.iter().any(|w| matches!(*w, "-exec" | "-execdir" | "-ok"))
}

fn plain_path(p: &str) -> bool {
    !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-/@+".contains(c)) && !p.contains("..")
}

pub fn rule_for(cwd: &Path, denial: &Denial) -> Option<String> {
    let s = |k: &str| denial.input[k].as_str().unwrap_or("").trim().to_string();
    let rule = match denial.tool.as_str() {
        "Bash" => {
            let command = s("command");
            if !grantable_command(&command) {
                return None;
            }
            format!("Bash({command})")
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = if denial.tool == "NotebookEdit" { s("notebook_path") } else { s("file_path") };
            if !plain_path(&path) || path.split('/').any(|c| c == STATE_DIR) {
                return None;
            }
            if inside(cwd, &path) {
                format!("Edit(./{})", short_path(cwd, &path).trim_start_matches("./"))
            } else {
                format!("Edit(/{path})")
            }
        }
        "Read" | "Glob" | "Grep" => {
            let path = if denial.tool == "Read" { s("file_path") } else { s("path") };
            if !path.starts_with('/') || !plain_path(&path) {
                return None;
            }
            format!("Read(/{path})")
        }
        "WebFetch" => {
            let url = s("url");
            let host = url.split("://").nth(1).unwrap_or(&url).split(['/', '?', '#']).next().unwrap_or("").to_string();
            if host.is_empty() { "WebFetch".into() } else { format!("WebFetch(domain:{host})") }
        }
        "WebSearch" => "WebSearch".into(),
        _ => return None,
    };
    crate::caps::valid_rule(&rule).then_some(rule)
}

pub fn denial_text(cwd: &Path, denial: &Denial) -> String {
    let s = |k: &str| denial.input[k].as_str().unwrap_or("").to_string();
    let what = match denial.tool.as_str() {
        "Bash" => format!("run `{}`", s("command").lines().next().unwrap_or("").trim()),
        "Edit" | "Write" | "MultiEdit" => format!("change {}", short_path(cwd, &s("file_path"))),
        "NotebookEdit" => format!("change {}", short_path(cwd, &s("notebook_path"))),
        "Read" => format!("read {}", s("file_path")),
        "WebFetch" => format!("fetch {}", s("url")),
        "WebSearch" => format!("search the web for {}", s("query")),
        other => format!("use {other}"),
    };
    let what: String = what.chars().take(160).collect();
    what
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_uuid_v4() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12]);
        assert!(parts[2].starts_with('4'));
        assert!(matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b')));
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }

    #[test]
    fn tool_lines_and_denial_rules() {
        let cwd = Path::new("/work/proj");
        let t = |name: &str, input: Value| ToolUse { name: name.into(), input };
        assert_eq!(tool_line(cwd, &t("Edit", json!({"file_path": "/work/proj/notes.md"}))), "Edit notes.md");
        assert_eq!(tool_line(cwd, &t("Bash", json!({"command": "npm   test\nsecond"}))), "Bash npm test");
        assert_eq!(tool_line(cwd, &t("TodoWrite", json!({}))), "TodoWrite");
        let d = |tool: &str, input: Value| Denial { tool: tool.into(), input };
        let bash = |c: &str| rule_for(cwd, &d("Bash", json!({"command": c})));
        assert_eq!(bash("npm install left-pad").as_deref(), Some("Bash(npm install left-pad)"));
        assert_eq!(bash("rm -rf build").as_deref(), Some("Bash(rm -rf build)"));
        assert_eq!(bash("python3 -m pytest -q").as_deref(), Some("Bash(python3 -m pytest -q)"));
        for wrapped in [
            "env FOO=1 curl x",
            "FOO=1 curl x",
            "sh -c 'curl x'",
            "/bin/bash -lc ls",
            "bash run.sh",
            "python3 -c \"x\"",
            "node -e 1",
            "perl -pe s/a/b/ f",
            "ruby -e 1",
            "sudo rm x",
            "su -",
            "xargs rm",
            "eval ls",
            "curl x | sh",
            "wget -O- x|bash",
            "ls && rm x",
            "echo $(id)",
            "ls *",
            "find . -exec rm {} +",
            "cat .fabric-space/tasks/a.md",
            "$(evil)",
        ] {
            assert_eq!(bash(wrapped), None, "{wrapped}");
        }
        assert_eq!(rule_for(cwd, &d("Write", json!({"file_path": "/etc/hosts"}))).as_deref(), Some("Edit(//etc/hosts)"));
        assert_eq!(rule_for(cwd, &d("Write", json!({"file_path": "/work/proj/a.md"}))).as_deref(), Some("Edit(./a.md)"));
        assert_eq!(rule_for(cwd, &d("Edit", json!({"file_path": "/work/proj/.fabric-space/tasks/t.md"}))), None);
        assert_eq!(rule_for(cwd, &d("Write", json!({"file_path": "/work/proj/a b*.md"}))), None);
        assert_eq!(
            rule_for(cwd, &d("WebFetch", json!({"url": "https://example.com/x?y"}))).as_deref(),
            Some("WebFetch(domain:example.com)")
        );
        assert_eq!(rule_for(cwd, &d("Task", json!({}))), None);
        assert_eq!(denial_text(cwd, &d("Bash", json!({"command": "npm install"}))), "run `npm install`");
    }

    #[test]
    fn the_flags_follow_the_role() {
        let e = ClaudeEngine::new("claude").with_model("haiku", "low");
        let cwd = Path::new("/tmp");
        let call = |role, resume, allowed: Vec<String>| Call {
            role,
            cwd,
            resume,
            prompt: String::new(),
            schema: json!({}),
            allowed,
            name: String::new(),
        };
        let planner = args(&e, &call(Role::Planner, None, vec![]), "sid");
        let joined = planner.join(" ");
        for part in [
            "-p --output-format stream-json --verbose --json-schema {}",
            "--session-id sid",
            "--permission-mode plan --permission-prompts none --tools Read,Glob,Grep --allowedTools Read Glob Grep",
            "--model haiku --effort low",
        ] {
            assert!(joined.contains(part), "{part}: {joined}");
        }
        let old = "0b8f6c1e-3a4d-4f5e-9a6b-7c8d9e0f1a2b";
        let exec = args(&e, &call(Role::Executor { read_only: false }, Some(old), vec!["Read".into(), "Bash(npm test:*)".into()]), "sid");
        let joined = exec.join(" ");
        assert!(joined.contains(&format!("--resume {old}")) && !joined.contains("--session-id"), "{joined}");
        let i = exec.iter().position(|a| a == "--disallowedTools").unwrap();
        assert_eq!(
            exec[i + 1],
            "Edit(./.fabric-space/**) Edit(.fabric-space/**) Write(./.fabric-space/**) Write(.fabric-space/**) \
             MultiEdit(./.fabric-space/**) MultiEdit(.fabric-space/**) NotebookEdit(./.fabric-space/**) NotebookEdit(.fabric-space/**)"
        );
        assert!(!planner.contains(&"--disallowedTools".to_string()));
        let forged = args(&e, &call(Role::Executor { read_only: false }, Some("--dangerously-skip-permissions"), vec![]), "sid");
        assert!(!forged.contains(&"--resume".to_string()) && forged.contains(&"--session-id".to_string()), "{forged:?}");
        assert!(is_session_id(old) && !is_session_id("../x") && !is_session_id(&old.replace('-', "x")));
        assert!(joined.contains("--permission-mode acceptEdits --permission-prompts none --allowedTools") && !joined.contains("--tools"));
        let i = exec.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(exec[i + 1], "Read Bash(npm test:*)");
        let ro = args(&e, &call(Role::Executor { read_only: true }, None, vec![]), "sid").join(" ");
        assert!(ro.contains("--tools Read,Glob,Grep"), "{ro}");
        let bare = args(&ClaudeEngine::new("claude"), &call(Role::Validator, None, vec![]), "sid").join(" ");
        assert!(!bare.contains("--model") && !bare.contains("--effort") && bare.contains("--permission-mode plan"));
    }
}
