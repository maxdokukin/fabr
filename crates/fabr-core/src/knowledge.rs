use crate::backend::local::atomic_write;
use crate::error::FabrError;
use crate::taskrec::{Question, TaskRecord};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const KINDS: [&str; 2] = ["rules", "knowledge"];
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_RECORDS: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeKnowledge {
    pub rules: String,
    pub knowledge: String,
    pub learned: Vec<Learned>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Learned {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub applies_when: String,
    #[serde(default = "project")]
    pub scope: String,
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default)]
    pub source_tasks: Vec<String>,
    #[serde(default)]
    pub last_confirmed: Option<String>,
}

fn project() -> String {
    "project".into()
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct LearnedFile {
    #[serde(default)]
    records: Vec<Learned>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KnowledgeReadReq {
    pub folder: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KnowledgeWriteReq {
    pub folder: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LearnedForgetReq {
    pub folder: String,
    pub id: String,
}

pub fn agents_dir(folder: &Path) -> PathBuf {
    folder.join(".fabric-space").join(".agents")
}

fn kind_path(folder: &Path, kind: &str) -> Result<PathBuf, FabrError> {
    if !KINDS.contains(&kind) {
        return Err(FabrError::BadRequest(format!("kind must be one of {}", KINDS.join(", "))));
    }
    Ok(agents_dir(folder).join(format!("{kind}.md")))
}

fn learned_path(folder: &Path) -> PathBuf {
    agents_dir(folder).join("learned.yaml")
}

fn read_text(path: &Path) -> Result<String, FabrError> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn read(folder: &Path) -> Result<NodeKnowledge, FabrError> {
    Ok(NodeKnowledge {
        rules: read_text(&kind_path(folder, "rules")?)?,
        knowledge: read_text(&kind_path(folder, "knowledge")?)?,
        learned: read_learned(folder)?,
    })
}

pub fn write(folder: &Path, kind: &str, text: &str) -> Result<NodeKnowledge, FabrError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(FabrError::BadRequest(format!("{kind} is over {} KB", MAX_FILE_BYTES / 1024)));
    }
    if !folder.is_dir() {
        return Err(FabrError::NotFound(format!("{} is not a folder", folder.display())));
    }
    let path = kind_path(folder, kind)?;
    std::fs::create_dir_all(agents_dir(folder))?;
    let mut body = text.trim_end().to_string();
    if !body.is_empty() {
        body.push('\n');
    }
    atomic_write(&path, body.as_bytes())?;
    read(folder)
}

pub fn read_learned(folder: &Path) -> Result<Vec<Learned>, FabrError> {
    let text = read_text(&learned_path(folder))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file: LearnedFile = serde_yaml::from_str(&text).map_err(|e| FabrError::Unprocessable(format!("learned.yaml: {e}")))?;
    Ok(file.records)
}

fn write_learned(folder: &Path, records: Vec<Learned>) -> Result<(), FabrError> {
    std::fs::create_dir_all(agents_dir(folder))?;
    let yaml = serde_yaml::to_string(&LearnedFile { records }).map_err(|e| FabrError::Io(e.to_string()))?;
    atomic_write(&learned_path(folder), yaml.as_bytes())
}

pub fn forget(folder: &Path, id: &str) -> Result<NodeKnowledge, FabrError> {
    let mut records = read_learned(folder)?;
    let before = records.len();
    records.retain(|r| r.id != id);
    if records.len() == before {
        return Err(FabrError::NotFound(format!("no learned record {id}")));
    }
    write_learned(folder, records)?;
    read(folder)
}

pub fn capture(folder: &Path, record_path: &Path, rec: &TaskRecord, at: &str) -> Result<usize, FabrError> {
    let answered: Vec<&Question> = rec.answers.iter().filter(|q| answer_text(q).is_some()).collect();
    if answered.is_empty() {
        return Ok(0);
    }
    let mut records = read_learned(folder)?;
    let source = record_path.to_string_lossy().to_string();
    let mut added = 0;
    for q in answered {
        let Some(text) = answer_text(q) else { continue };
        if text.to_ascii_lowercase().starts_with("allow") {
            continue;
        }
        let applies_when = one_line(&q.text);
        match records.iter_mut().find(|r| r.applies_when == applies_when && r.text == text) {
            Some(r) => {
                if !r.source_tasks.contains(&source) {
                    r.count += 1;
                    r.source_tasks.push(source.clone());
                }
                r.last_confirmed = Some(at.to_string());
            }
            None => {
                records.push(Learned {
                    id: next_id(&records),
                    text,
                    applies_when,
                    scope: project(),
                    count: 1,
                    source_tasks: vec![source.clone()],
                    last_confirmed: Some(at.to_string()),
                });
                added += 1;
            }
        }
    }
    if records.len() > MAX_RECORDS {
        records.drain(..records.len() - MAX_RECORDS);
    }
    write_learned(folder, records)?;
    Ok(added)
}

fn answer_text(q: &Question) -> Option<String> {
    let a = q.answer.as_ref()?;
    let value = a.replacement.as_deref().filter(|v| !v.trim().is_empty()).or(a.value.as_deref()).map(one_line)?;
    (!value.is_empty() && value.len() <= 400).then_some(value)
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn next_id(records: &[Learned]) -> String {
    let n = records.iter().filter_map(|r| r.id.strip_prefix('l').and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
    format!("l{}", n + 1)
}

pub fn chain(folder: &Path, root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(folder);
    while let Some(dir) = cur {
        out.push(dir.to_path_buf());
        if dir == root {
            break;
        }
        cur = dir.parent().filter(|p| p.starts_with(root));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskrec::Answer;

    fn answered(id: &str, text: &str, value: &str) -> Question {
        Question {
            id: id.into(),
            kind: "open".into(),
            text: text.into(),
            options: None,
            prior: None,
            fields: None,
            asked_at: None,
            answer: Some(Answer { value: Some(value.into()), replacement: None, granted: None, by: Some("gui".into()), at: None }),
        }
    }

    fn rec(answers: Vec<Question>) -> TaskRecord {
        let text = "---\nschema: 7\nid: x\ntitle: x\nstate: succeeded\nrequests: {delegate: false, cancel: false, redelegate: false}\ngrants: {}\nlinks: {}\n---\n## Context\n";
        let mut r = crate::taskrec::parse(text).unwrap();
        r.answers = answers;
        r
    }

    #[test]
    fn write_read_and_reject_unknown_kinds() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read(d.path()).unwrap(), NodeKnowledge::default());
        let k = write(d.path(), "rules", "- Ask before spending\n\n").unwrap();
        assert_eq!(k.rules, "- Ask before spending\n");
        assert!(d.path().join(".fabric-space/.agents/rules.md").is_file());
        assert!(matches!(write(d.path(), "../x", "y"), Err(FabrError::BadRequest(_))));
        assert_eq!(write(d.path(), "knowledge", "").unwrap().knowledge, "");
    }

    #[test]
    fn capture_adds_once_counts_repeats_and_skips_permission_answers() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.md");
        let b = d.path().join("b.md");
        let r = rec(vec![answered("q1", "Which shop?", "Prusa  store"), answered("q2", "Allow npm install?", "allow")]);
        assert_eq!(capture(d.path(), &a, &r, "2026-10-31T01:00:00Z").unwrap(), 1);
        assert_eq!(capture(d.path(), &a, &r, "2026-10-31T01:05:00Z").unwrap(), 0);
        assert_eq!(capture(d.path(), &b, &r, "2026-10-31T02:00:00Z").unwrap(), 0);
        let learned = read_learned(d.path()).unwrap();
        assert_eq!(learned.len(), 1);
        assert_eq!(
            (learned[0].id.as_str(), learned[0].text.as_str(), learned[0].applies_when.as_str()),
            ("l1", "Prusa store", "Which shop?")
        );
        assert_eq!((learned[0].count, learned[0].source_tasks.len()), (2, 2));
        assert_eq!(learned[0].last_confirmed.as_deref(), Some("2026-10-31T02:00:00Z"));
        assert!(forget(d.path(), "l1").unwrap().learned.is_empty());
        assert!(matches!(forget(d.path(), "l1"), Err(FabrError::NotFound(_))));
    }

    #[test]
    fn the_sample_learned_file_parses() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sample-fabr-space/home");
        let learned = read_learned(&dir).unwrap();
        assert_eq!((learned[0].id.as_str(), learned[0].scope.as_str()), ("g1", "global"));
    }

    #[test]
    fn chain_walks_up_to_the_root_only() {
        let root = Path::new("/a");
        assert_eq!(chain(Path::new("/a/b/c"), root), [PathBuf::from("/a/b/c"), "/a/b".into(), "/a".into()]);
        assert_eq!(chain(Path::new("/x/y"), root), [PathBuf::from("/x/y")]);
    }
}
