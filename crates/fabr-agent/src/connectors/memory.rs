//! Local learned preferences. This file is authoritative.
//!
//! Moss is a replaceable index and is not called from here. A missing Moss
//! credential leaves the index unwired; recall still reads this file.

use crate::now_utc;
use fabr_core::FabrError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LEARNED_FILE: &str = "learned-preferences.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub text: String,
    pub source: String,
    pub scope: String,
    pub status: String,
    pub task_id: String,
    pub created_at: String,
    pub confirmed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LearnedFile {
    version: u32,
    memories: Vec<Memory>,
}

impl LearnedFile {
    fn empty() -> Self {
        Self { version: 1, memories: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MossIndex {
    NotIntegrated,
}

pub fn moss_index_available(moss_key_present: bool) -> MossIndex {
    let _ = moss_key_present;
    MossIndex::NotIntegrated
}

pub fn learned_path(fabric_space: &Path) -> PathBuf {
    fabric_space.join(LEARNED_FILE)
}

pub fn memory_id(text: &str) -> String {
    format!("mem_{:016x}", fnv64(text.trim().to_lowercase().as_bytes()))
}

pub fn propose(fabric_space: &Path, text: &str, source: &str, scope: &str, task_id: &str) -> Result<Memory, FabrError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(FabrError::BadRequest("memory text is empty".into()));
    }
    let source = normalize_source(source)?;
    let scope = if scope.trim().is_empty() { "always" } else { scope.trim() };
    let mut file = load(fabric_space)?;
    let id = memory_id(text);
    let mut existing_status = None;
    if let Some(existing) = file.memories.iter_mut().find(|m| m.id == id) {
        if existing.status == "deleted" {
            existing.status = "proposed".into();
            existing.confirmed_at = None;
            existing.task_id = task_id.to_string();
            existing.source = source.into();
            existing.scope = scope.into();
            existing.text = text.to_string();
        }
        existing_status = Some(existing.status.clone());
    }
    if let Some(status) = existing_status {
        store(fabric_space, &file)?;
        return verified(fabric_space, &id, &status);
    }
    let memory = Memory {
        id,
        text: text.to_string(),
        source: source.to_string(),
        scope: scope.to_string(),
        status: "proposed".into(),
        task_id: task_id.to_string(),
        created_at: now_utc(),
        confirmed_at: None,
    };
    file.memories.push(memory.clone());
    store(fabric_space, &file)?;
    verified(fabric_space, &memory.id, "proposed")
}

pub fn confirm(fabric_space: &Path, id: &str, task_id: &str) -> Result<Memory, FabrError> {
    let mut file = load(fabric_space)?;
    let Some(memory) = file.memories.iter_mut().find(|m| m.id == id) else {
        return Err(FabrError::NotFound(format!("no memory {id}")));
    };
    if memory.status == "deleted" {
        return Err(FabrError::BadRequest(format!("memory {id} is deleted")));
    }
    memory.status = "confirmed".into();
    memory.task_id = task_id.to_string();
    if memory.confirmed_at.is_none() {
        memory.confirmed_at = Some(now_utc());
    }
    store(fabric_space, &file)?;
    verified(fabric_space, id, "confirmed")
}

pub fn delete(fabric_space: &Path, id: &str) -> Result<Memory, FabrError> {
    let mut file = load(fabric_space)?;
    let Some(memory) = file.memories.iter_mut().find(|m| m.id == id) else {
        return Err(FabrError::NotFound(format!("no memory {id}")));
    };
    memory.status = "deleted".into();
    let id = memory.id.clone();
    store(fabric_space, &file)?;
    verified(fabric_space, &id, "deleted")
}

pub fn recall(fabric_space: &Path, query: &str) -> Result<Vec<Memory>, FabrError> {
    let file = load(fabric_space)?;
    let query = query.trim().to_lowercase();
    Ok(file
        .memories
        .into_iter()
        .filter(|m| m.status == "confirmed")
        .filter(|m| query.is_empty() || m.text.to_lowercase().contains(&query))
        .collect())
}

pub fn recall_log(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return "recall_preferences source=local count=0".into();
    }
    let parts: Vec<String> = memories.iter().map(|m| format!("id={} source={} text={}", m.id, m.source, m.text)).collect();
    format!("recall_preferences source=local count={} {}", memories.len(), parts.join("; "))
}

fn normalize_source(source: &str) -> Result<&str, FabrError> {
    match source.trim() {
        "" | "explicit" => Ok("explicit"),
        "inferred" => Ok("inferred"),
        other => Err(FabrError::BadRequest(format!("unknown memory source {other}"))),
    }
}

fn verified(fabric_space: &Path, id: &str, status: &str) -> Result<Memory, FabrError> {
    load(fabric_space)?
        .memories
        .into_iter()
        .find(|m| m.id == id && m.status == status)
        .ok_or_else(|| FabrError::BadRequest(format!("memory {id} was not {status} after write")))
}

fn load(fabric_space: &Path) -> Result<LearnedFile, FabrError> {
    let path = learned_path(fabric_space);
    if !path.exists() {
        return Ok(LearnedFile::empty());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| FabrError::BadRequest(format!("learned preferences: {e}")))
}

fn store(fabric_space: &Path, file: &LearnedFile) -> Result<(), FabrError> {
    let path = learned_path(fabric_space);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(file).map_err(|e| FabrError::BadRequest(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut out = opts.open(&tmp).map_err(|e| FabrError::Io(format!("{}: {e}", tmp.display())))?;
        out.write_all(body.as_bytes()).map_err(|e| FabrError::Io(format!("{}: {e}", tmp.display())))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
    Ok(())
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: &str = "Stay under $100 for internship supplies";

    #[test]
    fn task_b_recalls_a_confirmed_budget_without_repeating_it() {
        let tmp = tempfile::tempdir().unwrap();
        let space = tmp.path();
        let proposed = propose(space, BUDGET, "explicit", "always", "task-a").unwrap();
        assert_eq!(proposed.status, "proposed");
        assert!(recall(space, "internship").unwrap().is_empty());
        confirm(space, &proposed.id, "task-a").unwrap();
        let query = "internship";
        assert!(!query.contains("$100"));
        let found = recall(space, query).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].text.contains("$100"));
        assert_eq!(found[0].source, "explicit");
        assert_eq!(found[0].task_id, "task-a");
        let log = recall_log(&found);
        assert!(log.contains("source=local"));
        assert!(log.contains("$100"));
        assert_eq!(moss_index_available(false), MossIndex::NotIntegrated);
        let names: Vec<_> = std::fs::read_dir(space).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec![LEARNED_FILE.to_string()]);
    }

    #[test]
    fn deleted_memory_is_not_recalled() {
        let tmp = tempfile::tempdir().unwrap();
        let space = tmp.path();
        let proposed = propose(space, BUDGET, "explicit", "always", "task-a").unwrap();
        confirm(space, &proposed.id, "task-a").unwrap();
        delete(space, &proposed.id).unwrap();
        assert!(recall(space, "").unwrap().is_empty());
        let again = propose(space, BUDGET, "explicit", "always", "task-c").unwrap();
        assert_eq!(again.status, "proposed");
        assert!(recall(space, "internship").unwrap().is_empty());
    }
}
