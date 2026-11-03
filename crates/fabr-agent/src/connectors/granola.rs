//! Read-only access to a local Granola meeting-notes cache.
//!
//! Granola is a macOS app that stores notes on disk. This connector never opens
//! a network connection and never sends data anywhere: it reads one local JSON
//! cache, extracts note fields heuristically, and returns a view. The cache path
//! comes from `FABR_GRANOLA_PATH`, falling back to the default macOS location
//! `~/Library/Application Support/Granola`. A missing or unreadable cache leaves
//! the tool empty rather than panicking. The cache content is untrusted data; it
//! is never applied to grants, approvals, or the tool list.

use fabr_core::FabrError;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Default macOS cache directory for Granola, relative to `$HOME`.
pub const DEFAULT_SUBPATH: &str = "Library/Application Support/Granola";
/// Cache file names Granola is known to write, newest first.
const CACHE_FILES: &[&str] = &["cache-v3.json", "cache-v2.json", "cache.json"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub body: String,
}

impl Note {
    fn snippet(&self, query: &str) -> String {
        let hay = self.body.to_lowercase();
        let needle = query.to_lowercase();
        let start = if needle.is_empty() { 0 } else { hay.find(&needle).unwrap_or(0) };
        // Back up to a char boundary a little before the match so the snippet has context.
        let from = self.body[..start.min(self.body.len())].char_indices().rev().take(40).last().map(|(i, _)| i).unwrap_or(0);
        let slice: String = self.body[from..].chars().take(160).collect();
        let slice = slice.replace(['\n', '\r'], " ");
        let trimmed = slice.trim();
        if trimmed.is_empty() { self.title.clone() } else { trimmed.to_string() }
    }
}

/// Resolve the cache file. `FABR_GRANOLA_PATH` may name either the cache file
/// directly or the Granola directory. Falls back to the default macOS path.
pub fn cache_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("FABR_GRANOLA_PATH") {
        let raw = raw.trim();
        if !raw.is_empty() {
            let p = PathBuf::from(raw);
            return Some(resolve_dir_or_file(&p));
        }
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.trim().is_empty())?;
    let dir = Path::new(&home).join(DEFAULT_SUBPATH);
    Some(resolve_dir_or_file(&dir))
}

fn resolve_dir_or_file(p: &Path) -> PathBuf {
    if p.is_dir() {
        for name in CACHE_FILES {
            let candidate = p.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
        // No known cache file present; return the first default name so the
        // caller's existence check reports a clean "not found".
        return p.join(CACHE_FILES[0]);
    }
    p.to_path_buf()
}

/// Load and parse the notes defensively. A missing file yields an empty list.
/// An unknown schema is walked heuristically; a parse error degrades to empty.
pub fn load_notes() -> Result<Vec<Note>, FabrError> {
    let Some(path) = cache_path() else {
        return Ok(Vec::new());
    };
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
    Ok(parse_notes(&text))
}

/// Parse notes from the raw cache text. Never panics; unknown shapes return
/// whatever note-like objects can be recovered, deepest-first de-duplicated by id.
pub fn parse_notes(text: &str) -> Vec<Note> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Granola sometimes stores the document as a JSON string containing JSON.
    let value: Value = match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::String(inner)) => serde_json::from_str::<Value>(&inner).unwrap_or(Value::String(inner)),
        Ok(other) => other,
        Err(_) => return Vec::new(),
    };
    let mut notes = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    collect(&value, &mut notes, &mut seen);
    notes
}

fn collect(value: &Value, out: &mut Vec<Note>, seen: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect(item, out, seen);
            }
        }
        Value::Object(map) => {
            #[allow(clippy::collapsible_if)]
            if let Some(note) = note_from_object(map) {
                if seen.insert(note.id.clone()) {
                    out.push(note);
                }
            }
            for child in map.values() {
                if child.is_object() || child.is_array() {
                    collect(child, out, seen);
                }
            }
        }
        _ => {}
    }
}

fn note_from_object(map: &serde_json::Map<String, Value>) -> Option<Note> {
    let title = first_str(map, &["title", "name", "heading", "subject"]);
    let body = first_str(map, &["text", "transcript", "notes", "content", "body", "summary", "markdown"]);
    // A note needs at least a title or a body to be worth surfacing.
    if title.is_none() && body.is_none() {
        return None;
    }
    let created_at =
        first_str(map, &["created_at", "createdAt", "created", "date", "updated_at", "updatedAt", "timestamp"]).unwrap_or_default();
    let title = title.unwrap_or_else(|| "(untitled)".to_string());
    let body = body.unwrap_or_default();
    let id = first_str(map, &["id", "document_id", "documentId", "uuid", "_id", "slug"]).unwrap_or_else(|| synth_id(&title, &created_at));
    Some(Note { id, title, created_at, body })
}

fn first_str(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        match map.get(*key) {
            Some(Value::String(s)) if !s.trim().is_empty() => return Some(s.trim().to_string()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn synth_id(title: &str, created_at: &str) -> String {
    format!("granola_{:016x}", fnv64(format!("{title}\n{created_at}").as_bytes()))
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// A markdown list of recent notes: title, date, id. `limit` caps the count.
pub fn granola_list(limit: usize) -> Result<String, FabrError> {
    let notes = load_notes()?;
    let limit = if limit == 0 { 20 } else { limit };
    let total = notes.len();
    let mut out = format!("source: granola\ntool: granola_list\ncount: {}\n", total.min(limit));
    if notes.is_empty() {
        out.push_str("notes: 0\n");
        return Ok(out);
    }
    for note in notes.iter().take(limit) {
        let date = if note.created_at.is_empty() { "unknown".to_string() } else { note.created_at.clone() };
        out.push_str(&format!("\n- title: {}\n  date: {}\n  id: {}\n", note.title, date, note.id));
    }
    Ok(out)
}

/// Case-insensitive substring search over titles and bodies.
pub fn granola_search(query: &str) -> Result<String, FabrError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(FabrError::BadRequest("granola_search needs a query".into()));
    }
    let needle = query.to_lowercase();
    let notes = load_notes()?;
    let matches: Vec<&Note> =
        notes.iter().filter(|n| n.title.to_lowercase().contains(&needle) || n.body.to_lowercase().contains(&needle)).collect();
    let mut out = format!("source: granola\ntool: granola_search\nquery: {query}\nmatches: {}\n", matches.len());
    for note in matches {
        out.push_str(&format!("\n- id: {}\n  title: {}\n  snippet: {}\n", note.id, note.title, note.snippet(query)));
    }
    Ok(out)
}

/// The full note text/transcript for one id.
pub fn granola_get(id: &str) -> Result<String, FabrError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(FabrError::BadRequest("granola_get needs an id".into()));
    }
    let notes = load_notes()?;
    let note = notes.into_iter().find(|n| n.id == id).ok_or_else(|| FabrError::NotFound(format!("no granola note {id}")))?;
    let date = if note.created_at.is_empty() { "unknown".to_string() } else { note.created_at.clone() };
    Ok(format!("source: granola\ntool: granola_get\nid: {}\ntitle: {}\ndate: {}\n\n{}\n", note.id, note.title, date, note.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "documents": [
            {
                "id": "doc-1",
                "title": "Weekly Sync",
                "created_at": "2026-10-20T09:00:00Z",
                "notes": "Discussed the internship budget and the launch timeline."
            },
            {
                "document_id": "doc-2",
                "name": "1:1 with Jordan",
                "createdAt": "2026-10-22T15:30:00Z",
                "transcript": "Jordan asked about the Granola connector rollout plan."
            },
            {
                "garbage": true
            }
        ],
        "meta": {"version": 3}
    }"#;

    #[test]
    fn parses_notes_from_mixed_field_names() {
        let notes = parse_notes(FIXTURE);
        assert_eq!(notes.len(), 2);
        let first = notes.iter().find(|n| n.id == "doc-1").unwrap();
        assert_eq!(first.title, "Weekly Sync");
        assert_eq!(first.created_at, "2026-10-20T09:00:00Z");
        assert!(first.body.contains("internship budget"));
        let second = notes.iter().find(|n| n.id == "doc-2").unwrap();
        assert_eq!(second.title, "1:1 with Jordan");
        assert!(second.body.contains("connector rollout"));
    }

    #[test]
    fn search_is_case_insensitive_over_titles_and_bodies() {
        let notes = parse_notes(FIXTURE);
        let hits: Vec<_> =
            notes.iter().filter(|n| n.title.to_lowercase().contains("sync") || n.body.to_lowercase().contains("sync")).collect();
        assert_eq!(hits.len(), 1);
        let budget: Vec<_> = notes.iter().filter(|n| n.body.to_lowercase().contains("budget")).collect();
        assert_eq!(budget.len(), 1);
        assert_eq!(budget[0].id, "doc-1");
        let snip = budget[0].snippet("budget");
        assert!(snip.to_lowercase().contains("budget"), "{snip}");
    }

    #[test]
    fn malformed_and_empty_degrade_without_panic() {
        assert!(parse_notes("").is_empty());
        assert!(parse_notes("not json at all").is_empty());
        assert!(parse_notes("{").is_empty());
        assert!(parse_notes("[1, 2, 3]").is_empty());
        assert!(parse_notes("{\"documents\": []}").is_empty());
    }

    #[test]
    fn handles_json_string_wrapped_cache() {
        // Granola can store the document as a JSON string containing JSON.
        let wrapped = serde_json::to_string(FIXTURE).unwrap();
        let notes = parse_notes(&wrapped);
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn missing_id_is_synthesized_and_stable() {
        let text = r#"{"title": "No Id Note", "text": "body here", "date": "2026-01-01"}"#;
        let a = parse_notes(text);
        let b = parse_notes(text);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, b[0].id);
        assert!(a[0].id.starts_with("granola_"));
    }

    #[test]
    fn env_override_points_at_a_fixture_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("cache-v3.json");
        std::fs::write(&file, FIXTURE).unwrap();
        // Directory override resolves to the cache file inside it.
        unsafe {
            std::env::set_var("FABR_GRANOLA_PATH", tmp.path());
        }
        let resolved = cache_path().unwrap();
        assert_eq!(resolved, file);
        let list = granola_list(10).unwrap();
        assert!(list.contains("Weekly Sync"));
        assert!(list.contains("doc-2"));
        let search = granola_search("jordan").unwrap();
        assert!(search.contains("doc-2"));
        let got = granola_get("doc-1").unwrap();
        assert!(got.contains("internship budget"));
        assert!(granola_get("missing").is_err());
        unsafe {
            std::env::remove_var("FABR_GRANOLA_PATH");
        }
    }
}
