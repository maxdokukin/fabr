use crate::error::FabrError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const TITLE_CHARS: usize = 80;
const TEXT_CHARS: usize = 2000;
const TOOL_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub modified_at: String,
    pub messages: usize,
    pub bytes: u64,
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

pub fn config_dir(home: &Path) -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home.join(".claude"),
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
        out.push(SessionSummary { id: id.to_string(), title: scan.title, modified_at, messages: scan.messages, bytes: meta.len() });
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
    let limit = limit.max(1);
    let truncated = entries.len() > limit;
    if truncated {
        entries.drain(..entries.len() - limit);
    }
    Ok(Transcript { id: id.to_string(), title: scan.title, entries, truncated })
}

struct Scan {
    title: String,
    messages: usize,
}

fn scan_lines(text: &str) -> Scan {
    let mut ai_title = None;
    let mut first_prompt = None;
    let mut messages = 0;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v.get("type").and_then(Value::as_str) {
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
    let title = ai_title.or(first_prompt).map(|t| clip(t.lines().next().unwrap_or("").trim(), TITLE_CHARS)).unwrap_or_default();
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
        let lines = [
            r#"{"type":"mode","mode":"normal"}"#.to_string(),
            r#"{"type":"user","message":{"role":"user","content":"Rename the scans\nby date"},"timestamp":"2026-10-30T10:00:00Z"}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"Looking."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls -la\nscans"}}]}}"#.into(),
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"denied"}]}}"#.into(),
            r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"hidden"}]}}"#.into(),
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/x/a.md"}},{"type":"text","text":"Done."}]}}"#.into(),
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
        assert_eq!((all[0].id.as_str(), all[0].title.as_str(), all[0].messages), (ID, "Rename the scans", 4));
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
                ("assistant", "Done.")
            ]
        );
        assert_eq!(t.entries[2].tool.as_deref(), Some("Bash"));
        assert!(!t.truncated);
        let short = transcript(home.path(), folder.path(), ID, 2).unwrap();
        assert!(short.truncated && short.entries.len() == 2 && short.entries[1].text == "Done.");
        assert!(matches!(transcript(home.path(), folder.path(), "../x", 5), Err(FabrError::BadRequest(_))));
        let other = "00000000-0000-0000-0000-000000000000";
        assert!(matches!(transcript(home.path(), folder.path(), other, 5), Err(FabrError::NotFound(_))));
    }

    #[test]
    fn an_ai_title_wins_over_the_first_prompt() {
        let text = format!("{}\n{}", r#"{"type":"user","message":{"content":"first"}}"#, r#"{"type":"ai-title","aiTitle":"Fix the CI"}"#);
        assert_eq!(scan_lines(&text).title, "Fix the CI");
        assert_eq!(clip("abcdef", 4), "abc…");
    }
}
