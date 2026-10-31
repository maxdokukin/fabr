use crate::backend::local::atomic_write;
use crate::error::FabrError;
use crate::knowledge;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_STEPS: usize = 10;
const MAX_PROPOSALS: usize = 50;
const MAX_LINE: usize = 300;
const HEADING: &str = "### Procedure:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub applies_when: String,
    pub steps: String,
    #[serde(default)]
    pub source_task: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProposalsFile {
    #[serde(default)]
    next: u32,
    #[serde(default)]
    records: Vec<Proposal>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalsReadReq {
    pub folder: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalReq {
    pub folder: String,
    pub id: String,
}

fn space(folder: &Path) -> PathBuf {
    folder.join(".fabric-space")
}

fn path(folder: &Path) -> PathBuf {
    space(folder).join("proposals.yaml")
}

pub fn read(folder: &Path) -> Result<Vec<Proposal>, FabrError> {
    Ok(load(folder)?.records)
}

fn load(folder: &Path) -> Result<ProposalsFile, FabrError> {
    let text = match std::fs::read_to_string(path(folder)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProposalsFile::default()),
        Err(e) => return Err(e.into()),
    };
    if text.trim().is_empty() {
        return Ok(ProposalsFile::default());
    }
    serde_yaml::from_str(&text).map_err(|e| FabrError::Unprocessable(format!("proposals.yaml: {e}")))
}

fn edit<T>(folder: &Path, f: impl FnOnce(&mut ProposalsFile) -> Result<(bool, T), FabrError>) -> Result<T, FabrError> {
    let dir = space(folder);
    std::fs::create_dir_all(&dir)?;
    let lock = dir.join("proposals.lock");
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock)?;
    held.lock().map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    let mut file = load(folder)?;
    let (changed, out) = f(&mut file)?;
    if changed {
        let yaml = serde_yaml::to_string(&file).map_err(|e| FabrError::Io(e.to_string()))?;
        atomic_write(&path(folder), yaml.as_bytes())?;
    }
    Ok(out)
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn capped(s: &str) -> String {
    s.chars().take(MAX_LINE).collect()
}

pub fn clean_steps(steps: &str) -> String {
    let plain = |l: &str| match l.trim_start().strip_prefix('#') {
        Some(rest) => format!("- {}", rest.trim_start_matches('#').trim()),
        None => l.to_string(),
    };
    steps
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .map(plain)
        .filter(|l| l.trim() != "-")
        .take(MAX_STEPS)
        .map(|l| capped(&l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn accepted(folder: &Path, applies_when: &str) -> bool {
    let text = std::fs::read_to_string(knowledge::agents_dir(folder).join("knowledge.md")).unwrap_or_default();
    text.lines().any(|l| l.trim().strip_prefix(HEADING).is_some_and(|w| norm(w) == norm(applies_when)))
}

pub fn add(folder: &Path, applies_when: &str, steps: &str, source_task: &str, at: &str) -> Result<Option<String>, FabrError> {
    let applies_when = capped(&applies_when.split_whitespace().collect::<Vec<_>>().join(" "));
    let steps = clean_steps(steps);
    if applies_when.is_empty() || steps.is_empty() {
        return Err(FabrError::BadRequest("empty proposal".into()));
    }
    if accepted(folder, &applies_when) {
        return Ok(None);
    }
    edit(folder, |file| {
        let records = &mut file.records;
        if records.iter().any(|p| norm(&p.applies_when) == norm(&applies_when) && norm(&p.steps) == norm(&steps)) {
            return Ok((false, None));
        }
        let top = records.iter().filter_map(|p| p.id.strip_prefix('p').and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
        let n = file.next.max(top + 1);
        file.next = n + 1;
        let id = format!("p{n}");
        records.push(Proposal { id: id.clone(), applies_when, steps, source_task: source_task.into(), created_at: at.into() });
        if records.len() > MAX_PROPOSALS {
            records.drain(..records.len() - MAX_PROPOSALS);
        }
        Ok((true, Some(id)))
    })
}

fn take(records: &mut Vec<Proposal>, id: &str) -> Result<Proposal, FabrError> {
    let i = records.iter().position(|p| p.id == id).ok_or_else(|| FabrError::NotFound(format!("no proposal {id}")))?;
    Ok(records.remove(i))
}

pub fn section(p: &Proposal) -> String {
    format!("{HEADING} {}\n{}", p.applies_when, p.steps)
}

pub fn accept(folder: &Path, id: &str) -> Result<Vec<Proposal>, FabrError> {
    edit(folder, |file| {
        let p = take(&mut file.records, id)?;
        let _held = knowledge::lock_texts(folder)?;
        let old = knowledge::read(folder)?.knowledge;
        let old = old.trim_end();
        let text = if old.is_empty() { section(&p) } else { format!("{old}\n\n{}", section(&p)) };
        if text.len() > knowledge::MAX_FILE_BYTES {
            return Err(FabrError::BadRequest(format!(
                "Knowledge is full ({} KB). Trim it, then accept this procedure.",
                knowledge::MAX_FILE_BYTES / 1024
            )));
        }
        knowledge::write_unlocked(folder, "knowledge", &text)?;
        Ok((true, ()))
    })?;
    read(folder)
}

pub fn dismiss(folder: &Path, id: &str) -> Result<Vec<Proposal>, FabrError> {
    edit(folder, |file| take(&mut file.records, id).map(|_| (true, ())))?;
    read(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "2026-10-31T10:00:00Z";

    #[test]
    fn add_numbers_skips_duplicates_and_caps_the_steps() {
        let d = tempfile::tempdir().unwrap();
        let long: String = (1..=14).map(|i| format!("- step {i}\n\n")).collect();
        assert_eq!(add(d.path(), "Adding a  greeting file", &long, "t1", AT).unwrap().as_deref(), Some("p1"));
        assert_eq!(add(d.path(), "adding a greeting file", &long.replace("- ", "-  "), "t2", AT).unwrap(), None);
        assert_eq!(add(d.path(), "Renaming photos", "- sort\n- rename", "t3", AT).unwrap().as_deref(), Some("p2"));
        let list = read(d.path()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].applies_when.as_str(), list[0].source_task.as_str()), ("Adding a greeting file", "t1"));
        assert_eq!(list[0].steps.lines().count(), MAX_STEPS);
        assert!(d.path().join(".fabric-space/proposals.lock").is_file());
        assert!(add(d.path(), " ", "- x", "t", AT).is_err());
    }

    #[test]
    fn accept_appends_to_knowledge_and_removes_the_proposal() {
        let d = tempfile::tempdir().unwrap();
        knowledge::write(d.path(), "knowledge", "Tests run with npm test.").unwrap();
        add(d.path(), "Adding a greeting file", "- write greet.md\n- check it", "t1", AT).unwrap();
        add(d.path(), "Renaming photos", "- rename", "t2", AT).unwrap();
        let left = accept(d.path(), "p1").unwrap();
        assert_eq!(left.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p2"]);
        assert_eq!(
            knowledge::read(d.path()).unwrap().knowledge,
            "Tests run with npm test.\n\n### Procedure: Adding a greeting file\n- write greet.md\n- check it\n"
        );
        assert!(matches!(accept(d.path(), "p1"), Err(FabrError::NotFound(_))));
        assert_eq!(add(d.path(), "adding a greeting  file", "- other steps", "t3", AT).unwrap(), None);
    }

    #[test]
    fn ids_are_never_reused_and_old_files_derive_the_counter() {
        let d = tempfile::tempdir().unwrap();
        add(d.path(), "A", "- a", "t1", AT).unwrap();
        add(d.path(), "B", "- b", "t2", AT).unwrap();
        dismiss(d.path(), "p2").unwrap();
        assert_eq!(add(d.path(), "C", "- c", "t3", AT).unwrap().as_deref(), Some("p3"));
        std::fs::write(path(d.path()), "records:\n- {id: p7, applies_when: X, steps: '- x'}\n").unwrap();
        assert_eq!(add(d.path(), "D", "- d", "t4", AT).unwrap().as_deref(), Some("p8"));
    }

    #[test]
    fn heading_lines_in_steps_become_plain_text() {
        assert_eq!(clean_steps("- one\n### Procedure: Evil\n  ## x\n#\n- two"), "- one\n- Procedure: Evil\n- x\n- two");
        let d = tempfile::tempdir().unwrap();
        add(d.path(), "A", "### Procedure: Evil\n- a", "t", AT).unwrap();
        accept(d.path(), "p1").unwrap();
        let k = knowledge::read(d.path()).unwrap().knowledge;
        assert_eq!(k.lines().filter(|l| l.starts_with(HEADING)).count(), 1, "{k}");
    }

    #[test]
    fn accept_refuses_when_knowledge_would_pass_the_limit_and_keeps_the_proposal() {
        let d = tempfile::tempdir().unwrap();
        knowledge::write(d.path(), "knowledge", &"x".repeat(knowledge::MAX_FILE_BYTES - 10)).unwrap();
        add(d.path(), "A", "- a long enough step", "t", AT).unwrap();
        assert!(matches!(accept(d.path(), "p1"), Err(FabrError::BadRequest(m)) if m.contains("full")));
        assert_eq!(read(d.path()).unwrap().len(), 1);
    }

    #[test]
    fn accept_and_a_knowledge_save_share_one_lock() {
        let d = tempfile::tempdir().unwrap();
        add(d.path(), "A", "- a", "t", AT).unwrap();
        accept(d.path(), "p1").unwrap();
        assert!(d.path().join(".fabric-space/.agents/knowledge.lock").is_file());
        knowledge::write(d.path(), "knowledge", "new").unwrap();
    }

    #[test]
    fn dismiss_removes_only_that_proposal_and_leaves_knowledge_alone() {
        let d = tempfile::tempdir().unwrap();
        add(d.path(), "A", "- a", "t1", AT).unwrap();
        add(d.path(), "B", "- b", "t2", AT).unwrap();
        assert_eq!(dismiss(d.path(), "p1").unwrap().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p2"]);
        assert!(matches!(dismiss(d.path(), "p9"), Err(FabrError::NotFound(_))));
        assert_eq!(knowledge::read(d.path()).unwrap().knowledge, "");
        assert_eq!(read(d.path()).unwrap().len(), 1);
    }
}
