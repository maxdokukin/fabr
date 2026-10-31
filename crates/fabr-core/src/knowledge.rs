use crate::backend::local::atomic_write;
use crate::error::FabrError;
use crate::taskrec::{Question, TaskRecord};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

pub const KINDS: [&str; 2] = ["rules", "knowledge"];
pub(crate) const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_RECORDS: usize = 200;
const WALK_DEPTH: usize = 8;
const WALK_DIRS: usize = 20_000;
const WALK_ENTRIES: usize = 200_000;
const HEAVY_DIRS: [&str; 7] = ["node_modules", "target", "Library", "Applications", "Pictures", "Music", "Movies"];
const MAX_LEARNED_BYTES: u64 = 256 * 1024;
pub const GLOBAL: &str = "global";

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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub always: bool,
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

#[derive(Debug, Clone, Deserialize)]
pub struct LearnedAlwaysReq {
    pub folder: String,
    pub id: String,
    pub always: bool,
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

pub(crate) fn lock_texts(folder: &Path) -> Result<std::fs::File, FabrError> {
    let dir = agents_dir(folder);
    std::fs::create_dir_all(&dir)?;
    let lock = dir.join("knowledge.lock");
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock)?;
    held.lock().map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    Ok(held)
}

pub fn write(folder: &Path, kind: &str, text: &str) -> Result<NodeKnowledge, FabrError> {
    if !folder.is_dir() {
        return Err(FabrError::NotFound(format!("{} is not a folder", folder.display())));
    }
    let _held = lock_texts(folder)?;
    write_unlocked(folder, kind, text)
}

pub(crate) fn write_unlocked(folder: &Path, kind: &str, text: &str) -> Result<NodeKnowledge, FabrError> {
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

fn edit_learned<T>(folder: &Path, f: impl FnOnce(&mut Vec<Learned>) -> Result<(bool, T), FabrError>) -> Result<T, FabrError> {
    let dir = agents_dir(folder);
    std::fs::create_dir_all(&dir)?;
    let lock = dir.join("learned.lock");
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock)?;
    held.lock().map_err(|e| FabrError::Io(format!("{}: {e}", lock.display())))?;
    let mut records = read_learned(folder)?;
    let (changed, out) = f(&mut records)?;
    if changed {
        write_learned(folder, records)?;
    }
    Ok(out)
}

fn no_record(id: &str) -> FabrError {
    FabrError::NotFound(format!("no learned record {id}"))
}

pub fn forget(folder: &Path, id: &str) -> Result<NodeKnowledge, FabrError> {
    if !learned_path(folder).is_file() {
        return Err(no_record(id));
    }
    edit_learned(folder, |records| {
        let before = records.len();
        records.retain(|r| r.id != id);
        if records.len() == before { Err(no_record(id)) } else { Ok((true, ())) }
    })?;
    read(folder)
}

pub fn set_always(folder: &Path, id: &str, always: bool) -> Result<NodeKnowledge, FabrError> {
    if !learned_path(folder).is_file() {
        return Err(no_record(id));
    }
    let off = edit_learned(folder, |records| {
        let Some(r) = records.iter_mut().find(|r| r.id == id) else { return Err(no_record(id)) };
        let changed = r.always != always;
        r.always = always;
        if changed && always {
            r.last_confirmed = Some(now_utc());
        }
        Ok((changed, (!always && r.scope == GLOBAL).then(|| r.clone())))
    })?;
    if let Some(g) = off {
        clear_always(&projects(folder), &g);
    }
    read(folder)
}

fn clear_always(dirs: &[PathBuf], like: &Learned) -> Vec<String> {
    let mut out = Vec::new();
    for dir in dirs {
        if !read_learned(dir).unwrap_or_default().iter().any(|r| r.always && same(r, like)) {
            continue;
        }
        let cleared = edit_learned(dir, |records| {
            let ids: Vec<String> = records
                .iter_mut()
                .filter(|r| r.always && same(r, like))
                .map(|r| {
                    r.always = false;
                    r.id.clone()
                })
                .collect();
            Ok((!ids.is_empty(), ids))
        });
        out.extend(cleared.unwrap_or_default());
    }
    out
}

fn now_utc() -> String {
    format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
}

fn key(r: &Learned) -> (String, String) {
    (one_line(&r.applies_when).to_lowercase(), one_line(&r.text).to_lowercase())
}

pub fn same(a: &Learned, b: &Learned) -> bool {
    key(a) == key(b)
}

pub fn stale(r: &Learned, days: u32, now: &str) -> bool {
    let date = |s: &str| chrono::NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok();
    let (Some(last), Some(now)) = (r.last_confirmed.as_deref().and_then(date), date(now)) else { return false };
    r.always && days > 0 && (now - last).num_days() > i64::from(days)
}

pub fn review(chain: &[PathBuf], like: &Learned, keep: bool, days: u32, now: &str) -> Vec<String> {
    if !keep {
        let mut dirs = chain.to_vec();
        let global = chain.iter().any(|d| read_learned(d).unwrap_or_default().iter().any(|r| r.scope == GLOBAL && same(r, like)));
        if global && let Some(root) = chain.last() {
            dirs.extend(projects(root).into_iter().filter(|d| !chain.contains(d)));
        }
        return clear_always(&dirs, like);
    }
    let mut changed = Vec::new();
    for dir in chain {
        if !read_learned(dir).unwrap_or_default().iter().any(|r| same(r, like) && stale(r, days, now)) {
            continue;
        }
        let refreshed = edit_learned(dir, |records| {
            let ids: Vec<String> = records
                .iter_mut()
                .filter(|r| same(r, like) && stale(r, days, now))
                .map(|r| {
                    r.last_confirmed = Some(now.to_string());
                    r.id.clone()
                })
                .collect();
            Ok((!ids.is_empty(), ids))
        });
        changed.extend(refreshed.unwrap_or_default());
    }
    changed
}

pub fn announce(space: &Path, path: &Path, msg: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(space)?;
    let line = crate::redact::redact(&format!("{} learned {} {}\n", now_utc(), path.display(), one_line(msg)));
    std::fs::OpenOptions::new().create(true).append(true).open(space.join("events.log"))?.write_all(line.as_bytes())
}

fn is_node(dir: &Path) -> bool {
    dir.join(".fabric-space").is_dir() || dir.join(crate::node::NODE_DIR).exists()
}

fn projects(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let (mut nodes, mut rest) = (VecDeque::from([(root.to_path_buf(), 0)]), VecDeque::new());
    let (mut dirs, mut entries) = (0, 0);
    while let Some((dir, depth)) = nodes.pop_front().or_else(|| rest.pop_front()) {
        dirs += 1;
        if dirs > WALK_DIRS || entries > WALK_ENTRIES {
            break;
        }
        if dir != root && std::fs::metadata(learned_path(&dir)).is_ok_and(|m| m.is_file() && m.len() <= MAX_LEARNED_BYTES) {
            out.push(dir.clone());
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten().filter(|_| depth < WALK_DEPTH) {
            entries += 1;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = e.path();
            if is_node(&path) {
                nodes.push_back((path, depth + 1));
            } else if !HEAVY_DIRS.contains(&name.as_str()) {
                rest.push_back((path, depth + 1));
            }
        }
    }
    out.sort();
    out
}

pub fn promote(root: &Path, folder: &Path, id: &str) -> Option<(Learned, String)> {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (root, folder) = (canon(root), canon(folder));
    if folder == root || !folder.starts_with(&root) {
        return None;
    }
    let r = read_learned(&folder).ok()?.into_iter().find(|r| r.id == id && r.always && r.scope != GLOBAL)?;
    let mut copies = vec![(folder.clone(), r.clone())];
    for dir in projects(&root).into_iter().filter(|d| *d != folder) {
        if let Some(o) = read_learned(&dir).unwrap_or_default().into_iter().find(|o| o.always && o.scope != GLOBAL && same(o, &r)) {
            copies.push((dir, o));
        }
    }
    if copies.len() < 2 {
        return None;
    }
    let (g, fresh) = edit_learned(&root, |records| {
        let fresh = !records.iter().any(|g| g.scope == GLOBAL && same(g, &r));
        if fresh {
            let n = records.iter().filter_map(|g| g.id.strip_prefix('g').and_then(|n| n.parse::<u32>().ok())).max().unwrap_or(0);
            let g = Learned {
                id: format!("g{}", n + 1),
                scope: GLOBAL.into(),
                count: 0,
                source_tasks: Vec::new(),
                last_confirmed: None,
                always: true,
                ..r.clone()
            };
            records.push(g);
        }
        let Some(g) = records.iter_mut().find(|g| g.scope == GLOBAL && same(g, &r)) else { return Ok((false, None)) };
        let mut sum = 0;
        for (_, c) in &copies {
            sum += c.count;
            for s in &c.source_tasks {
                if !g.source_tasks.contains(s) {
                    g.source_tasks.push(s.clone());
                }
            }
            g.last_confirmed = g.last_confirmed.clone().max(c.last_confirmed.clone());
        }
        g.count = g.count.max(sum);
        Ok((true, Some((g.clone(), fresh))))
    })
    .ok()??;
    let mut names: Vec<String> = copies.iter().map(|(d, _)| d.strip_prefix(&root).unwrap_or(d).to_string_lossy().into_owned()).collect();
    names.sort();
    let last = names.pop().unwrap_or_default();
    let names = if names.is_empty() { last } else { format!("{} and {last}", names.join(", ")) };
    fresh.then(|| (g, format!("promoted {id} to global (confirmed in {names})")))
}

pub fn find(chain: &[PathBuf], pick: impl Fn(&Learned) -> bool) -> Option<(PathBuf, Learned)> {
    chain.iter().find_map(|dir| read_learned(dir).unwrap_or_default().into_iter().find(|r| pick(r)).map(|r| (dir.clone(), r)))
}

pub fn find_cited(chain: &[PathBuf], id: &str, shown: impl Fn(&Learned) -> bool) -> Option<(PathBuf, Learned)> {
    let hits: Vec<(PathBuf, Learned)> =
        chain.iter().flat_map(|d| read_learned(d).unwrap_or_default().into_iter().filter(|r| r.id == id).map(|r| (d.clone(), r))).collect();
    hits.iter().find(|(_, r)| shown(r)).or(hits.first()).cloned()
}

fn ids_in_use(chain: &[PathBuf]) -> Vec<Learned> {
    let mut dirs = chain.to_vec();
    if let Some(root) = chain.last() {
        dirs.extend(projects(root).into_iter().filter(|d| !chain.contains(d)));
    }
    dirs.iter().flat_map(|d| read_learned(d).unwrap_or_default()).collect()
}

pub fn capture(chain: &[PathBuf], record_path: &Path, rec: &TaskRecord, at: &str) -> Result<usize, FabrError> {
    let Some(folder) = chain.first() else { return Ok(0) };
    let source = record_path.to_string_lossy().to_string();
    let mut found: Vec<(String, String)> = Vec::new();
    for q in rec.answers.iter().filter(|q| q.kind == "choice") {
        let (Some(prior), Some(a)) = (&q.prior, &q.answer) else { continue };
        let hit = find(chain, |r| r.text == prior.text);
        if a.value.as_deref() == Some("no") {
            let text = one_line(a.replacement.as_deref().unwrap_or(""));
            if !text.is_empty() && text.len() <= 400 {
                found.push((hit.map_or_else(|| one_line(&q.text), |(_, r)| r.applies_when), text));
            }
            continue;
        }
        let Some((dir, r)) = hit else { continue };
        if dir != *folder {
            edit_learned(&dir, |others| {
                let Some(o) = others.iter_mut().find(|o| o.id == r.id) else { return Ok((false, ())) };
                confirm(o, &source, at);
                Ok((true, ()))
            })?;
        } else {
            found.push((r.applies_when.clone(), r.text.clone()));
        }
    }
    for q in rec.answers.iter().filter(|q| q.kind != "choice") {
        let Some(text) = answer_text(q) else { continue };
        if !text.to_ascii_lowercase().starts_with("allow") {
            found.push((one_line(&q.text), text));
        }
    }
    if found.is_empty() {
        return Ok(0);
    }
    let taken = ids_in_use(chain);
    edit_learned(folder, |records| {
        let mut added = 0;
        for (applies_when, text) in found {
            match records.iter_mut().find(|r| r.applies_when == applies_when && r.text == text) {
                Some(r) => confirm(r, &source, at),
                None => {
                    let id = next_id(&[records.as_slice(), taken.as_slice()].concat());
                    records.push(Learned {
                        id,
                        text,
                        applies_when,
                        scope: project(),
                        count: 1,
                        source_tasks: vec![source.clone()],
                        last_confirmed: Some(at.to_string()),
                        always: false,
                    });
                    added += 1;
                }
            }
        }
        if records.len() > MAX_RECORDS {
            records.drain(..records.len() - MAX_RECORDS);
        }
        Ok((true, added))
    })
}

fn confirm(r: &mut Learned, source: &str, at: &str) {
    if !r.source_tasks.iter().any(|s| s == source) {
        r.count += 1;
        r.source_tasks.push(source.to_string());
    }
    r.last_confirmed = Some(at.to_string());
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
        let c = [d.path().to_path_buf()];
        assert_eq!(capture(&c, &a, &r, "2026-10-31T01:00:00Z").unwrap(), 1);
        assert_eq!(capture(&c, &a, &r, "2026-10-31T01:05:00Z").unwrap(), 0);
        assert_eq!(capture(&c, &b, &r, "2026-10-31T02:00:00Z").unwrap(), 0);
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

    fn chose(value: &str, prior: &str, replacement: Option<&str>) -> Question {
        let mut q = answered("q2", "Ship it home again?", value);
        q.kind = "choice".into();
        q.prior = Some(crate::taskrec::Prior { text: prior.into(), source_task: None, last_confirmed: None });
        q.answer.as_mut().unwrap().replacement = replacement.map(String::from);
        q
    }

    #[test]
    fn set_always_round_trips_and_is_written_only_when_true() {
        let d = tempfile::tempdir().unwrap();
        let c = [d.path().to_path_buf()];
        capture(&c, &d.path().join("a.md"), &rec(vec![answered("q1", "Which shop?", "Prusa")]), "2026-10-31T01:00:00Z").unwrap();
        let path = d.path().join(".fabric-space/.agents/learned.yaml");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("always"));
        assert!(set_always(d.path(), "l1", true).unwrap().learned[0].always);
        assert!(std::fs::read_to_string(&path).unwrap().contains("always: true"));
        assert!(!set_always(d.path(), "l1", false).unwrap().learned[0].always);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("always"));
        assert!(matches!(set_always(d.path(), "l9", true), Err(FabrError::NotFound(_))));
    }

    #[test]
    fn choice_answers_confirm_the_prior_or_capture_the_replacement() {
        let d = tempfile::tempdir().unwrap();
        let c = [d.path().to_path_buf()];
        capture(&c, &d.path().join("a.md"), &rec(vec![answered("q1", "Where to ship?", "Home")]), "2026-10-31T01:00:00Z").unwrap();
        assert_eq!(capture(&c, &d.path().join("b.md"), &rec(vec![chose("once", "Home", None)]), "2026-10-31T02:00:00Z").unwrap(), 0);
        let l = read_learned(d.path()).unwrap();
        assert_eq!((l.len(), l[0].count, l[0].last_confirmed.as_deref()), (1, 2, Some("2026-10-31T02:00:00Z")));
        let no = rec(vec![chose("no", "Home", Some("the  office")), chose("no", "Home", None)]);
        assert_eq!(capture(&c, &d.path().join("c.md"), &no, "2026-10-31T03:00:00Z").unwrap(), 1);
        let l = read_learned(d.path()).unwrap();
        assert_eq!((l[1].id.as_str(), l[1].text.as_str(), l[1].applies_when.as_str()), ("l2", "the office", "Where to ship?"));
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn the_sample_learned_file_parses() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sample-fabr-space/home");
        let learned = read_learned(&dir).unwrap();
        assert_eq!((learned[0].id.as_str(), learned[0].scope.as_str(), learned[0].always), ("g1", "global", false));
    }

    #[test]
    fn new_ids_do_not_collide_with_ids_up_the_chain() {
        let root = tempfile::tempdir().unwrap();
        let child = root.path().join("c");
        std::fs::create_dir_all(&child).unwrap();
        let r = rec(vec![answered("q1", "Which shop?", "Prusa")]);
        capture(&[root.path().to_path_buf()], &root.path().join("a.md"), &r, "2026-10-31T01:00:00Z").unwrap();
        let r2 = rec(vec![answered("q1", "Which colour?", "black")]);
        capture(&[child.clone(), root.path().to_path_buf()], &child.join("b.md"), &r2, "2026-10-31T01:00:00Z").unwrap();
        assert_eq!(read_learned(root.path()).unwrap()[0].id, "l1");
        assert_eq!(read_learned(&child).unwrap()[0].id, "l2");
    }

    fn always_in(dir: &Path, id: &str, text: &str, when: &str, count: u32, source: &str) {
        let r = Learned {
            id: id.into(),
            text: text.into(),
            applies_when: when.into(),
            scope: project(),
            count,
            source_tasks: vec![source.into()],
            last_confirmed: Some("2026-10-01T00:00:00Z".into()),
            always: true,
        };
        let mut records = read_learned(dir).unwrap();
        records.push(r);
        write_learned(dir, records).unwrap();
    }

    #[test]
    fn an_always_rule_confirmed_in_two_projects_is_promoted_to_global_once() {
        let root = tempfile::tempdir().unwrap();
        let (pets, notes, deep) = (root.path().join("pets"), root.path().join("team-notes"), root.path().join("node_modules/x"));
        for d in [&pets, &notes, &deep] {
            std::fs::create_dir_all(d).unwrap();
        }
        always_in(&pets, "l3", "Ship to  home", "Where to ship?", 2, "/p/a.md");
        assert_eq!(promote(root.path(), &pets, "l3"), None);
        always_in(&deep, "l1", "Ship to home", "Where to ship?", 5, "/d/a.md");
        assert_eq!(promote(root.path(), &pets, "l3"), None);
        always_in(&notes, "l1", "ship to HOME", "where to  ship?", 3, "/n/b.md");
        let (g, msg) = promote(root.path(), &pets, "l3").unwrap();
        assert_eq!(msg, "promoted l3 to global (confirmed in pets and team-notes)");
        assert_eq!((g.id.as_str(), g.scope.as_str(), g.always, g.count), ("g1", "global", true, 5));
        assert_eq!(g.source_tasks, ["/p/a.md", "/n/b.md"]);
        assert_eq!(read_learned(root.path()).unwrap(), [g]);
        assert_eq!(promote(root.path(), &notes, "l1"), None);
        assert_eq!(read_learned(root.path()).unwrap().len(), 1);
        assert_eq!(read_learned(&pets).unwrap()[0].scope, "project");
    }

    #[test]
    fn different_text_or_a_rule_that_is_not_always_is_not_promoted() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        always_in(&a, "l1", "Ship to home", "Where to ship?", 1, "/a.md");
        always_in(&b, "l1", "Ship to the office", "Where to ship?", 1, "/b.md");
        always_in(&b, "l2", "Ship to home", "Which box?", 1, "/b.md");
        assert_eq!(promote(root.path(), &a, "l1"), None);
        set_always(&a, "l1", false).unwrap();
        assert_eq!(promote(root.path(), &a, "l1"), None);
        assert!(read_learned(root.path()).unwrap().is_empty());
    }

    #[test]
    fn an_always_rule_is_stale_after_the_review_days_and_review_refreshes_or_clears_it() {
        let d = tempfile::tempdir().unwrap();
        always_in(d.path(), "l1", "Ship to home", "Where to ship?", 1, "/a.md");
        let r = &read_learned(d.path()).unwrap()[0];
        assert!(!stale(r, 90, "2026-12-30T00:00:00Z") && stale(r, 90, "2026-12-31T00:00:00Z"));
        assert!(!stale(r, 0, "2030-01-01T00:00:00Z"));
        let c = [d.path().to_path_buf()];
        let like = Learned { text: "ship  to HOME".into(), ..r.clone() };
        assert!(review(&c, &like, true, 90, "2026-11-01T00:00:00Z").is_empty());
        assert_eq!(review(&c, &like, true, 30, "2026-12-01T00:00:00Z"), ["l1"]);
        assert_eq!(read_learned(d.path()).unwrap()[0].last_confirmed.as_deref(), Some("2026-12-01T00:00:00Z"));
        assert_eq!(review(&c, &like, false, 30, "2026-12-02T00:00:00Z"), ["l1"]);
        assert!(!read_learned(d.path()).unwrap()[0].always);
    }

    #[test]
    fn new_ids_are_unique_under_the_root_and_a_cited_id_resolves_by_its_text() {
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let r = root.path().to_path_buf();
        capture(&[a.clone(), r.clone()], &a.join("t1.md"), &rec(vec![answered("q1", "Which shop?", "Shop A")]), "2026-10-31T00:00:00Z")
            .unwrap();
        capture(&[b.clone(), r.clone()], &b.join("t2.md"), &rec(vec![answered("q1", "Which box?", "Blue")]), "2026-10-31T00:00:00Z")
            .unwrap();
        capture(std::slice::from_ref(&r), &r.join("t3.md"), &rec(vec![answered("q1", "Which carrier?", "UPS")]), "2026-10-31T00:00:00Z")
            .unwrap();
        let ids: Vec<String> = [&a, &b, &r].iter().map(|d| read_learned(d).unwrap()[0].id.clone()).collect();
        assert_eq!(ids, ["l1", "l2", "l3"]);
        always_in(&r, "l1", "UPS", "Which carrier?", 1, "/old.md");
        let chain = [a.clone(), r.clone()];
        assert_eq!(find_cited(&chain, "l1", |x| x.text == "UPS").unwrap().0, r);
        assert_eq!(find_cited(&chain, "l1", |_| false).unwrap().0, a);
        assert!(find_cited(&chain, "l9", |_| true).is_none());
    }

    #[test]
    fn concurrent_captures_into_one_folder_lose_nothing() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let r = rec(vec![answered("q1", &format!("Question {i}?"), "yes please")]);
                    capture(std::slice::from_ref(&dir), &dir.join(format!("t{i}.md")), &r, "2026-10-31T00:00:00Z").unwrap();
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let mut ids: Vec<String> = read_learned(&dir).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 8);
        ids.dedup();
        assert_eq!(ids.len(), 8);
        assert!(agents_dir(&dir).join("learned.lock").is_file());
    }

    #[test]
    fn no_or_a_global_turned_off_clears_always_on_every_copy() {
        let root = tempfile::tempdir().unwrap();
        let (pets, notes) = (root.path().join("pets"), root.path().join("notes"));
        for d in [&pets, &notes] {
            std::fs::create_dir_all(d).unwrap();
            always_in(d, "l1", "Ship to home", "Where to ship?", 1, "/a.md");
            always_in(d, "l2", "Ship to home", "Which box?", 1, "/a.md");
        }
        always_in(root.path(), "l3", "ship to HOME", "where to ship?", 1, "/b.md");
        let chain = [pets.clone(), root.path().to_path_buf()];
        let like = read_learned(&pets).unwrap()[0].clone();
        assert_eq!(review(&chain, &like, false, 90, "2026-10-02T00:00:00Z"), ["l1", "l3"]);
        assert!(read_learned(&notes).unwrap().iter().all(|r| r.always));
        assert!(read_learned(&pets).unwrap()[1].always);
        let mut g = read_learned(root.path()).unwrap();
        g[0].scope = GLOBAL.into();
        g[0].always = true;
        write_learned(root.path(), g).unwrap();
        assert!(review(&chain, &like, false, 90, "2026-10-02T00:00:00Z").contains(&"l1".to_string()));
        assert!(!read_learned(&notes).unwrap()[0].always && read_learned(&notes).unwrap()[1].always);
        set_always(&notes, "l1", true).unwrap();
        set_always(root.path(), "l3", true).unwrap();
        set_always(root.path(), "l3", false).unwrap();
        assert!(!read_learned(&notes).unwrap()[0].always && read_learned(&notes).unwrap()[1].always);
    }

    #[test]
    fn the_walk_skips_heavy_folders_unless_they_are_nodes() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["Library/x", "target/y", "Music", "a/b/c"] {
            always_in(&root.path().join(dir), "l1", "x", "y", 1, "/a.md");
        }
        let found: Vec<PathBuf> = projects(root.path()).iter().map(|p| p.strip_prefix(root.path()).unwrap().to_path_buf()).collect();
        assert_eq!(found, [PathBuf::from("Music"), "a/b/c".into()]);
    }

    #[test]
    fn chain_walks_up_to_the_root_only() {
        let root = Path::new("/a");
        assert_eq!(chain(Path::new("/a/b/c"), root), [PathBuf::from("/a/b/c"), "/a/b".into(), "/a".into()]);
        assert_eq!(chain(Path::new("/x/y"), root), [PathBuf::from("/x/y")]);
    }
}
