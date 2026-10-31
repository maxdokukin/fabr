use crate::error::FabrError;
use crate::taskrec::{self, State, Step, TaskRecord};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

const SPACE: &str = ".fabric-space";
const BACKEND: &str = "_backend.yaml";
const SKIP: [&str; 2] = ["node_modules", "target"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub path: String,
    pub mtime: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FabricListReq {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub home: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FabricListRes {
    pub records: Vec<FileStamp>,
    pub registry: Vec<FileStamp>,
    pub events: Option<FileStamp>,
    #[serde(default)]
    pub backend: Option<FileStamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricReadReq {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricReadRes {
    pub text: String,
    pub mtime: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricWriteReq {
    pub path: String,
    pub text: String,
    pub expected_updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricWriteRes {
    pub mtime: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FabricStampReq {
    pub roots: Vec<String>,
    #[serde(default)]
    pub home: Option<String>,
}

fn mtime_of(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

fn stat(p: &Path) -> Option<FileStamp> {
    let m = std::fs::metadata(p).ok().filter(|m| m.is_file())?;
    Some(FileStamp { path: p.to_string_lossy().into_owned(), mtime: mtime_of(&m) })
}

fn files_with_ext(dir: &Path, ext: &str, out: &mut Vec<FileStamp>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == ext)
            && !e.file_name().to_string_lossy().starts_with('.')
            && let Some(s) = stat(&p)
        {
            out.push(s);
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<FileStamp>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name == SPACE {
            files_with_ext(&e.path().join("tasks"), "md", out);
        } else if !name.starts_with('.') && !SKIP.contains(&name.as_ref()) {
            walk(&e.path(), out);
        }
    }
}

pub fn list(req: &FabricListReq) -> Result<FabricListRes, FabrError> {
    list_in(req, &crate::setup::Ctx::from_env().home)
}

pub fn list_in(req: &FabricListReq, user_home: &Path) -> Result<FabricListRes, FabrError> {
    let mut res = FabricListRes::default();
    if !req.root.is_empty() {
        let root = &crate::paths::expand_home(&req.root, user_home);
        if !root.is_dir() {
            return Err(FabrError::NotFound(format!("not a folder: {}", req.root)));
        }
        files_with_ext(&root.join(SPACE).join("tasks"), "md", &mut res.records);
        walk(root, &mut res.records);
        res.records.sort_by(|a, b| a.path.cmp(&b.path));
        res.records.dedup_by(|a, b| a.path == b.path);
    }
    if let Some(h) = req.home.as_deref().filter(|h| !h.is_empty()) {
        let space = Path::new(h).join(SPACE);
        files_with_ext(&space.join("registry"), "yaml", &mut res.registry);
        res.registry.retain(|f| !f.path.ends_with(BACKEND));
        res.registry.sort_by(|a, b| a.path.cmp(&b.path));
        res.events = stat(&space.join("events.log"));
        res.backend = stat(&space.join("registry").join(BACKEND));
    }
    Ok(res)
}

pub fn stamp(req: &FabricStampReq) -> Result<String, FabrError> {
    stamp_in(req, &crate::setup::Ctx::from_env().home)
}

pub fn stamp_in(req: &FabricStampReq, user_home: &Path) -> Result<String, FabrError> {
    let mut files: Vec<FileStamp> = Vec::new();
    let mut add = |l: FabricListRes| {
        files.extend(l.records);
        files.extend(l.registry);
        files.extend(l.events);
        files.extend(l.backend);
    };
    for (i, r) in req.roots.iter().enumerate() {
        if let Ok(l) = list_in(&FabricListReq { root: r.clone(), home: if i == 0 { req.home.clone() } else { None } }, user_home) {
            add(l);
        }
    }
    if req.roots.is_empty()
        && let Ok(l) = list(&FabricListReq { root: String::new(), home: req.home.clone() })
    {
        add(l);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let mut h = sha1_smol::Sha1::new();
    for f in &files {
        let body = std::fs::read(&f.path).unwrap_or_default();
        let content = sha1_smol::Sha1::from(&body).digest();
        h.update(format!("{}\0{}:{}:{}\n", f.path, f.mtime, body.len(), content).as_bytes());
    }
    Ok(h.digest().to_string())
}

fn checked(path: &str) -> Result<PathBuf, FabrError> {
    let p = Path::new(path);
    let inside = path.contains(&format!("/{SPACE}/"));
    if !inside || !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(FabrError::BadRequest(format!("only absolute paths inside a {SPACE} folder can be used: {path}")));
    }
    Ok(p.to_path_buf())
}

fn writable(p: &Path) -> bool {
    let parent = p.parent();
    let kind = parent.and_then(|d| d.file_name()).and_then(|n| n.to_str());
    let space = parent.and_then(|d| d.parent()).and_then(|d| d.file_name()).and_then(|n| n.to_str());
    let ext = p.extension().and_then(|e| e.to_str());
    space == Some(SPACE) && matches!((kind, ext), (Some("tasks"), Some("md")) | (Some("registry"), Some("yaml")))
}

pub fn read(req: &FabricReadReq) -> Result<FabricReadRes, FabrError> {
    let p = checked(&req.path)?;
    let m = std::fs::metadata(&p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => FabrError::NotFound(format!("no such file: {}", req.path)),
        _ => FabrError::Io(format!("{}: {e}", req.path)),
    })?;
    if !m.is_file() {
        return Err(FabrError::BadRequest(format!("not a file: {}", req.path)));
    }
    let text = std::fs::read_to_string(&p).map_err(|e| FabrError::Io(format!("{}: {e}", req.path)))?;
    Ok(FabricReadRes { text, mtime: mtime_of(&m) })
}

pub fn updated_at(text: &str) -> String {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return String::new();
    }
    for l in lines {
        if l.trim_end() == "---" {
            break;
        }
        if let Some(v) = l.strip_prefix("updated_at:") {
            return v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        }
    }
    String::new()
}

fn task_markdown(p: &Path) -> bool {
    p.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str()) == Some("tasks") && p.extension().and_then(|e| e.to_str()) == Some("md")
}

/// Keep fields the GUI does not own, then let the schema writer check
/// `updated_at` and render the bytes.
pub(crate) fn gui_record(disk: &TaskRecord, incoming: &TaskRecord) -> Result<TaskRecord, FabrError> {
    let by_hand = disk.state == State::Reminder && incoming.state == State::Succeeded;
    if incoming.state != disk.state && !by_hand {
        return Err(FabrError::BadRequest("the gui writes state only to mark a reminder done".into()));
    }
    if incoming.step != disk.step {
        return Err(FabrError::BadRequest("the gui does not write step".into()));
    }
    if let Some(until) = &incoming.suspend_until
        && !taskrec::valid_due(until)
    {
        return Err(FabrError::BadRequest(format!("suspend_until must be YYYY-MM-DD or YYYY-MM-DDTHH:MM, got {until:?}")));
    }
    if incoming.paused != disk.paused {
        return Err(FabrError::BadRequest("the gui does not write paused".into()));
    }
    let same = |a: &str, b: &str| taskrec::normalize_section(a) == taskrec::normalize_section(b);
    let changed = [
        !same(&incoming.context, &disk.context),
        !same(&incoming.spec, &disk.spec),
        !same(&incoming.success, &disk.success),
        !same(&incoming.plan, &disk.plan),
    ];
    let [context_changed, spec_changed, success_changed, plan_changed] = changed;
    let assertions_changed = incoming.assertions != disk.assertions;
    let content_changed = changed.iter().any(|c| *c)
        || assertions_changed
        || incoming.grants.requested != disk.grants.requested
        || incoming.title != disk.title
        || incoming.due != disk.due
        || incoming.pri != disk.pri;
    let editable = matches!(disk.state, State::Reminder | State::Task | State::Defined);
    if !editable && content_changed {
        return Err(FabrError::BadRequest("task is frozen once execution starts".into()));
    }
    let mut rec = incoming.clone();
    if matches!(disk.state, State::Succeeded) {
        rec.suspended = disk.suspended;
        rec.suspend_until = disk.suspend_until.clone();
    }
    if matches!(disk.state, State::Succeeded | State::Failed) {
        rec.requests.pause = disk.requests.pause;
        rec.requests.resume = disk.requests.resume;
        rec.requests.complete = disk.requests.complete;
    }
    rec.schema = disk.schema;
    rec.id = disk.id.clone();
    rec.source = disk.source.clone();
    rec.waiting = disk.waiting;
    rec.paused = disk.paused;
    rec.created_at = disk.created_at.clone();
    rec.answers = disk.answers.clone();
    rec.plan_meta = disk.plan_meta.clone();
    rec.verdict = if by_hand { None } else { disk.verdict.clone() };
    rec.cost_usd = disk.cost_usd;
    rec.links = disk.links.clone();
    rec.by = disk.by.clone();
    rec.needs = disk.needs.clone();
    rec.log = disk.log.clone();
    rec.sessions = disk.sessions.clone();
    if rec.question.as_ref().map(|q| &q.id) != disk.question.as_ref().map(|q| &q.id) {
        rec.question = disk.question.clone();
    }
    let digest = |r: &TaskRecord| r.approval.as_ref().and_then(|a| a.digest.clone());
    if digest(incoming).is_some_and(|d| Some(d) != digest(disk)) {
        return Err(FabrError::Conflict { message: "the plan changed since you opened it; review it again".into(), node: None });
    }
    let keep = |changed: bool, kept: &str, new: &mut String| {
        if !changed {
            *new = kept.to_string();
        }
    };
    keep(context_changed, &disk.context, &mut rec.context);
    keep(spec_changed, &disk.spec, &mut rec.spec);
    keep(success_changed, &disk.success, &mut rec.success);
    keep(plan_changed, &disk.plan, &mut rec.plan);
    let human = Some(taskrec::Author::Human);
    if context_changed {
        rec.by.context = human;
    }
    if spec_changed {
        rec.by.spec = human;
    }
    if success_changed || assertions_changed {
        rec.by.success = human;
    }
    if plan_changed {
        rec.by.plan = human;
    }
    let past_define = matches!(disk.step, Some(Step::Plan | Step::Approve));
    if (spec_changed || success_changed || assertions_changed) && past_define && incoming.requests.redo.is_none() && rec.by.plan != human {
        rec.requests.redo = Some(taskrec::RedoStage::Plan);
    }
    if by_hand {
        if !rec.log.is_empty() && !rec.log.ends_with('\n') {
            rec.log.push('\n');
        }
        rec.log.push_str(&format!("- {} done: marked done by hand\n", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")));
    }
    if let Some(kept) = &disk.approval {
        let plan_side = spec_changed || success_changed || plan_changed || assertions_changed;
        if plan_side {
            let digest = taskrec::approval_digest(&rec);
            rec.approval = Some(taskrec::Approval { status: "pending".into(), by: None, at: None, note: None, digest: Some(digest) });
            rec.waiting = disk.state == State::Defined;
        } else {
            match rec.approval.as_mut() {
                Some(approval) => approval.digest = kept.digest.clone(),
                None => rec.approval = Some(kept.clone()),
            }
        }
    }
    Ok(rec)
}

pub fn write(req: &FabricWriteReq) -> Result<FabricWriteRes, FabrError> {
    let p = checked(&req.path)?;
    if !writable(&p) {
        return Err(FabrError::BadRequest(format!("only tasks/*.md and registry/*.yaml can be written: {}", req.path)));
    }
    let old = std::fs::read_to_string(&p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => FabrError::NotFound(format!("no such file: {}", req.path)),
        _ => FabrError::Io(format!("{}: {e}", req.path)),
    })?;
    if task_markdown(&p) {
        let disk = taskrec::parse(&old)?;
        let incoming = taskrec::parse(&req.text)?;
        let rec = gui_record(&disk, &incoming)?;
        taskrec::write_record(&p, &rec, Some(&req.expected_updated_at))?;
    } else {
        let current = updated_at(&old);
        if current != req.expected_updated_at {
            return Err(FabrError::Conflict {
                message: format!("{} changed on disk: updated_at is {current:?}, expected {:?}", req.path, req.expected_updated_at),
                node: None,
            });
        }
        let dir = p.parent().ok_or_else(|| FabrError::BadRequest("no parent folder".into()))?;
        let io = |e: std::io::Error| FabrError::Io(format!("{}: {e}", req.path));
        let mut tmp = tempfile::Builder::new().prefix(".fabr-write-").suffix(".tmp").tempfile_in(dir).map_err(io)?;
        tmp.write_all(req.text.as_bytes()).map_err(io)?;
        tmp.as_file().sync_all().map_err(io)?;
        if let Ok(m) = std::fs::metadata(&p) {
            let _ = std::fs::set_permissions(tmp.path(), m.permissions());
        }
        tmp.persist(&p).map_err(|e| io(e.error))?;
    }
    let io = |e: std::io::Error| FabrError::Io(format!("{}: {e}", req.path));
    let m = std::fs::metadata(&p).map_err(io)?;
    Ok(FabricWriteRes { mtime: mtime_of(&m) })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricCreateReq {
    pub folder: String,
    pub title: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub pri: Option<u8>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub success: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FabricCreateRes {
    pub path: String,
    pub id: String,
}

pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(40);
    let out = out.trim_matches('-');
    if out.is_empty() { "task".into() } else { out.into() }
}

fn yaml_title(s: &str) -> String {
    let plain = s.chars().all(|c| c.is_ascii_alphanumeric() || " _.,/+-".contains(c))
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !s.ends_with(' ')
        && s.parse::<f64>().is_err()
        && !["null", "true", "false", "yes", "no", "on", "off"].contains(&s.to_ascii_lowercase().as_str());
    if plain {
        return s.to_string();
    }
    let esc: String = s
        .chars()
        .flat_map(|c| match c {
            '\\' => vec!['\\', '\\'],
            '"' => vec!['\\', '"'],
            c => vec![c],
        })
        .collect();
    format!("\"{esc}\"")
}

fn section_text(t: &Option<String>) -> String {
    let t = t.as_deref().unwrap_or("").trim();
    if t.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = t.lines().map(|l| if l.starts_with("## ") { format!(" {l}") } else { l.to_string() }).collect();
    format!("{}\n", lines.join("\n"))
}

pub fn create(req: &FabricCreateReq) -> Result<FabricCreateRes, FabrError> {
    create_in(req, &crate::setup::Ctx::from_env().home)
}

pub(crate) struct NewTask {
    pub(crate) title: String,
    due: Option<String>,
    pri: u8,
}

pub(crate) fn clean_title(s: &str) -> String {
    s.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect::<String>().trim().to_string()
}

pub(crate) fn validate_new(req: &FabricCreateReq) -> Result<NewTask, FabrError> {
    if req.title.trim().contains('\n') {
        return Err(FabrError::BadRequest("a task needs a one-line title".into()));
    }
    let title = clean_title(&req.title);
    if title.is_empty() {
        return Err(FabrError::BadRequest("a task needs a one-line title".into()));
    }
    let due = req.due.as_deref().map(str::trim).filter(|d| !d.is_empty());
    if let Some(d) = due
        && !taskrec::valid_due(d)
    {
        return Err(FabrError::BadRequest(format!("due must be YYYY-MM-DD or YYYY-MM-DDTHH:MM: {d}")));
    }
    let pri = req.pri.unwrap_or(0);
    if pri > 3 {
        return Err(FabrError::BadRequest(format!("pri must be 0-3: {pri}")));
    }
    Ok(NewTask { title, due: due.map(String::from), pri })
}

pub(crate) fn new_task_base(title: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    format!("{}-{}", now.format("%Y-%m-%d"), slugify(title))
}

pub(crate) fn render_new(req: &FabricCreateReq, nt: &NewTask, id: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let stamp = now.format("%Y-%m-%dT%H:%M:%SZ");
    let pri = nt.pri;
    format!(
        "---\nschema: 7\nid: {id}\nsource: null\ntitle: {}\ndue: {}\npri: {pri}\nstate: reminder\nstep: null\nwaiting: false\npaused: false\nsuspended: false\nsuspend_until: null\ncreated_at: {stamp}\nupdated_at: {stamp}\nrequests: {{delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null}}\nquestion: null\nanswers: []\ngrants: {{requested: [], granted: []}}\nplan_meta: null\nassertions: []\napproval: null\nverdict: null\ncost_usd: 0\nlinks: {{redelegated_from: null, redelegated_to: null}}\nby: {{context: null, spec: null, success: null, plan: null}}\nneeds: null\nsessions: {{plan: null, execute: null, evaluate: null}}\n---\n## Context\n{}## Spec\n{}## Success\n{}## Plan\n## Log\n",
        yaml_title(&nt.title),
        nt.due.as_deref().unwrap_or("null"),
        section_text(&req.context),
        section_text(&req.spec),
        section_text(&req.success),
    )
}

pub fn create_in(req: &FabricCreateReq, user_home: &Path) -> Result<FabricCreateRes, FabrError> {
    let nt = validate_new(req)?;
    let folder = crate::paths::expand_home(req.folder.trim(), user_home);
    if !folder.is_absolute() || !folder.is_dir() {
        return Err(FabrError::NotFound(format!("not a folder: {}", req.folder)));
    }
    let dir = folder.join(SPACE).join("tasks");
    let io = |e: std::io::Error| FabrError::Io(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(&dir).map_err(io)?;
    let now = chrono::Utc::now();
    let base = new_task_base(&nt.title, now);
    let mut n = 1;
    loop {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let path = dir.join(format!("{id}.md"));
        if path.exists() {
            n += 1;
            continue;
        }
        let text = render_new(req, &nt, &id, now);
        let io = |e: std::io::Error| FabrError::Io(format!("{}: {e}", path.display()));
        let mut tmp = tempfile::Builder::new().prefix(".fabr-write-").suffix(".tmp").tempfile_in(&dir).map_err(io)?;
        tmp.write_all(text.as_bytes()).map_err(io)?;
        tmp.as_file().sync_all().map_err(io)?;
        match tmp.persist_noclobber(&path) {
            Ok(_) => return Ok(FabricCreateRes { path: path.to_string_lossy().into_owned(), id }),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(io(e.error)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportTodosReq {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportTodosRes {
    pub created: usize,
}

pub fn front_matter_items(folder: &Path) -> Vec<crate::item::Item> {
    crate::item::items_in(&crate::fs::LocalFs::new(folder.to_path_buf()), "")
}

pub fn import_todos(req: &ImportTodosReq) -> Result<ImportTodosRes, FabrError> {
    import_todos_in(req, &crate::setup::Ctx::from_env().home)
}

pub fn import_todos_in(req: &ImportTodosReq, user_home: &Path) -> Result<ImportTodosRes, FabrError> {
    let folder = crate::paths::expand_home(req.path.trim(), user_home);
    if !folder.is_absolute() || !folder.is_dir() {
        return Err(FabrError::NotFound(format!("not a folder: {}", req.path)));
    }
    let xs = folder.join(crate::node::NODE_DIR);
    let todo = xs.join(crate::node::LEGACY_TODO_FILE);
    let text = match std::fs::read(&todo) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ImportTodosRes { created: 0 }),
        Err(e) => return Err(FabrError::Io(format!("{}: {e}", todo.display()))),
    };
    let mut created = 0;
    for line in crate::todo::parse_legacy(&text).into_iter().filter(|l| !l.done) {
        let item = FabricCreateReq {
            folder: folder.to_string_lossy().into_owned(),
            title: line.title,
            due: line.due,
            pri: Some(line.pri),
            context: Some("Imported from .fabr/.todo".into()),
            spec: None,
            success: None,
        };
        create_in(&item, user_home)?;
        created += 1;
    }
    let mut target = xs.join(".todo.imported");
    let mut n = 1;
    while target.exists() {
        n += 1;
        target = xs.join(format!(".todo.imported.{n}"));
    }
    std::fs::rename(&todo, &target).map_err(|e| FabrError::Io(format!("{}: {e}", todo.display())))?;
    Ok(ImportTodosRes { created })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(updated: &str) -> String {
        format!(
            "---\nschema: 2\nid: a\nsource: null\ntitle: t\nstate: reminder\nstep: null\nwaiting: false\ncreated_at: T0\nupdated_at: {updated}\nrequests:\n  delegate: false\n  cancel: false\n  redelegate: false\ngrants:\n  requested: []\n  granted: []\nlinks:\n  redelegated_from: null\n  redelegated_to: null\n---\n## Context\n"
        )
    }

    fn tree() -> (tempfile::TempDir, String, String) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("proj");
        for (rel, body) in [
            ("proj/.fabric-space/tasks/a.md", rec("T1")),
            ("proj/sub/deep/.fabric-space/tasks/b.md", rec("T1")),
            ("proj/.fabric-space/tasks/note.txt", "x".into()),
            ("proj/.fabric-space/.agents/knowledge.md", "k".into()),
            ("proj/node_modules/x/.fabric-space/tasks/c.md", rec("T1")),
            ("proj/.hidden/.fabric-space/tasks/d.md", rec("T1")),
            ("proj/other/readme.md", "r".into()),
            ("home/.fabric-space/registry/r1.yaml", "a: 1".into()),
            ("home/.fabric-space/registry/r1.txt", "a: 1".into()),
            ("home/.fabric-space/events.log", "e".into()),
        ] {
            let f = d.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
        }
        let home = d.path().join("home").to_string_lossy().into_owned();
        (d, root.to_string_lossy().into_owned(), home)
    }

    #[test]
    fn list_finds_nested_tasks_only() {
        let (_d, root, home) = tree();
        let r = list(&FabricListReq { root, home: Some(home) }).unwrap();
        let names: Vec<_> = r.records.iter().map(|f| f.path.rsplit('/').next().unwrap().to_string()).collect();
        assert_eq!(names, ["a.md", "b.md"]);
        assert_eq!(r.registry.len(), 1);
        assert!(r.events.unwrap().path.ends_with("events.log"));
    }

    #[test]
    fn read_refuses_outside_fabric_space() {
        let (d, root, _) = tree();
        let outside = format!("{root}/other/readme.md");
        assert!(matches!(read(&FabricReadReq { path: outside }), Err(FabrError::BadRequest(_))));
        let dotdot = format!("{root}/.fabric-space/../other/readme.md");
        assert!(read(&FabricReadReq { path: dotdot }).is_err());
        let ok = read(&FabricReadReq { path: format!("{root}/.fabric-space/tasks/a.md") }).unwrap();
        assert!(ok.text.contains("updated_at: T1"));
        drop(d);
    }

    #[test]
    fn write_checks_updated_at() {
        let (_d, root, _) = tree();
        let path = format!("{root}/.fabric-space/tasks/a.md");
        let stale = FabricWriteReq { path: path.clone(), text: rec("T3"), expected_updated_at: "T0".into() };
        let e = write(&stale).unwrap_err();
        assert!(matches!(e, FabrError::Conflict { .. }));
        assert_eq!(updated_at(&std::fs::read_to_string(&path).unwrap()), "T1");
        write(&FabricWriteReq { path: path.clone(), text: rec("T2"), expected_updated_at: "T1".into() }).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(updated_at(&written), "T2");
        assert_eq!(written, taskrec::render(&taskrec::parse(&written).unwrap()));
        let left: Vec<_> = std::fs::read_dir(format!("{root}/.fabric-space/tasks")).unwrap().flatten().collect();
        assert_eq!(left.len(), 2);
    }

    #[test]
    fn registry_yaml_stays_raw_text() {
        let (_d, _root, home) = tree();
        let path = format!("{home}/.fabric-space/registry/r1.yaml");
        let text = "task_id: t\n";
        write(&FabricWriteReq { path: path.clone(), text: text.into(), expected_updated_at: String::new() }).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn gui_cannot_write_state() {
        let (_d, root, _) = tree();
        let path = format!("{root}/.fabric-space/tasks/a.md");
        let mut incoming = taskrec::parse(&rec("T1")).unwrap();
        incoming.state = taskrec::State::Task;
        let err =
            write(&FabricWriteReq { path: path.clone(), text: taskrec::render(&incoming), expected_updated_at: "T1".into() }).unwrap_err();
        assert!(err.message().contains("state"));
        assert_eq!(taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap().state, taskrec::State::Reminder);
    }

    #[test]
    fn gui_sets_suspension_and_cannot_set_paused() {
        let (_d, root, _) = tree();
        let path = format!("{root}/.fabric-space/tasks/a.md");
        let put = |r: &taskrec::TaskRecord, at: &str| {
            write(&FabricWriteReq { path: path.clone(), text: taskrec::render(r), expected_updated_at: at.into() })
        };
        let mut incoming = taskrec::parse(&rec("T1")).unwrap();
        incoming.suspended = true;
        incoming.suspend_until = Some("2999-01-01T09:30".into());
        incoming.requests.pause = true;
        put(&incoming, "T1").unwrap();
        let disk = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(disk.suspended && disk.requests.pause && disk.suspend_until.as_deref() == Some("2999-01-01T09:30"));
        let mut bad = disk.clone();
        bad.suspend_until = Some("tomorrow".into());
        assert!(put(&bad, &disk.updated_at).unwrap_err().message().contains("suspend_until"));
        let mut bad = disk.clone();
        bad.paused = true;
        assert!(put(&bad, &disk.updated_at).unwrap_err().message().contains("paused"));
        let mut ask = disk.clone();
        ask.requests.complete = true;
        put(&ask, &disk.updated_at).unwrap();
        let disk = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(disk.requests.complete && !disk.paused);
    }

    #[test]
    fn write_refuses_other_files() {
        let (_d, root, home) = tree();
        for p in [
            format!("{root}/.fabric-space/tasks/note.txt"),
            format!("{root}/.fabric-space/.agents/knowledge.md"),
            format!("{home}/.fabric-space/events.log"),
        ] {
            let r = write(&FabricWriteReq { path: p, text: "x".into(), expected_updated_at: String::new() });
            assert!(matches!(r, Err(FabrError::BadRequest(_))));
        }
    }

    #[test]
    fn stamp_tracks_changes() {
        let (_d, root, home) = tree();
        let req = FabricStampReq { roots: vec![root.clone()], home: Some(home) };
        let s1 = stamp(&req).unwrap();
        assert_eq!(s1, stamp(&req).unwrap());
        write(&FabricWriteReq {
            path: format!("{root}/.fabric-space/tasks/a.md"),
            text: rec("T2 longer"),
            expected_updated_at: "T1".into(),
        })
        .unwrap();
        assert_ne!(s1, stamp(&req).unwrap());
    }

    fn put(p: &str, body: &str) {
        std::fs::create_dir_all(Path::new(p).parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn tilde_roots_resolve_through_the_given_home() {
        let (d, _root, _) = tree();
        let uh = d.path();
        let req = FabricListReq { root: "~/proj".into(), home: None };
        assert!(list_in(&req, uh).unwrap().records.iter().any(|r| r.path.ends_with("a.md")));
        assert!(list_in(&FabricListReq { root: "~".into(), home: None }, uh).is_ok());
        assert!(list_in(&FabricListReq { root: "~/nope".into(), home: None }, uh).is_err());
        let st = FabricStampReq { roots: vec!["~/proj".into()], home: None };
        assert_ne!(stamp_in(&st, uh).unwrap(), stamp_in(&FabricStampReq { roots: vec![], home: None }, uh).unwrap());
    }

    #[test]
    fn list_without_root_returns_home_files_only() {
        let (_d, _root, home) = tree();
        let r = list(&FabricListReq { root: String::new(), home: Some(home) }).unwrap();
        assert!(r.records.is_empty());
        assert_eq!(r.registry.len(), 1);
        assert!(r.events.is_some());
        let none: FabricListReq = serde_json::from_str(r#"{"home":"/nope"}"#).unwrap();
        assert!(none.root.is_empty());
    }

    #[test]
    fn backend_heartbeat_is_reported_and_kept_out_of_registry() {
        let (_d, root, home) = tree();
        let req = FabricListReq { root, home: Some(home.clone()) };
        assert!(list(&req).unwrap().backend.is_none());
        put(&format!("{home}/.fabric-space/registry/_backend.yaml"), "heartbeat_at: 2026-10-24T18:00:00Z\n");
        let r = list(&req).unwrap();
        assert!(r.backend.unwrap().path.ends_with("registry/_backend.yaml"));
        assert_eq!(r.registry.len(), 1);
        assert!(r.registry.iter().all(|f| !f.path.ends_with("_backend.yaml")));
    }

    #[test]
    fn stamp_sees_same_length_same_mtime_edit() {
        let (_d, root, home) = tree();
        let req = FabricStampReq { roots: vec![root.clone()], home: Some(home) };
        let path = format!("{root}/.fabric-space/tasks/a.md");
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let s1 = stamp(&req).unwrap();
        std::fs::write(&path, rec("T9")).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(before).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert_ne!(s1, stamp(&req).unwrap());
    }

    #[test]
    fn stamp_covers_home_without_roots_and_the_heartbeat() {
        let (_d, _root, home) = tree();
        let req = FabricStampReq { roots: vec![], home: Some(home.clone()) };
        let s1 = stamp(&req).unwrap();
        put(&format!("{home}/.fabric-space/registry/_backend.yaml"), "heartbeat_at: x\n");
        assert_ne!(s1, stamp(&req).unwrap());
    }

    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let t = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &t)
            } else {
                std::fs::copy(e.path(), t).unwrap();
            }
        }
    }

    #[test]
    fn fixture_round_trip() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/05-running");
        let d = tempfile::tempdir().unwrap();
        copy(&src, d.path());
        let (root, home) = (d.path().join("project").to_string_lossy().into_owned(), d.path().join("home").to_string_lossy().into_owned());
        let l = list(&FabricListReq { root, home: Some(home) }).unwrap();
        assert_eq!(l.records.len(), 1);
        assert_eq!(l.registry.len(), 1);
        let path = l.records[0].path.clone();
        let r = read(&FabricReadReq { path: path.clone() }).unwrap();
        let cur = updated_at(&r.text);
        assert_eq!(cur, "2026-10-24T18:59:40Z");
        assert_eq!(cur, taskrec::parse(&r.text).unwrap().updated_at);
        let text = r.text.replace(&cur, "2026-10-24T19:00:00Z");
        write(&FabricWriteReq { path: path.clone(), text, expected_updated_at: cur.clone() }).unwrap();
        let written = read(&FabricReadReq { path: path.clone() }).unwrap().text;
        let parsed = taskrec::parse(&written).unwrap();
        assert_eq!(updated_at(&written), "2026-10-24T19:00:00Z");
        assert_eq!(parsed.updated_at, "2026-10-24T19:00:00Z");
        assert_eq!(parsed.state, taskrec::State::InProgress);
        assert!(parsed.log.contains("2026-10-24T18:59:40Z execute: selected standard shipping"));
        assert_eq!(written, taskrec::render(&parsed));
        assert!(write(&FabricWriteReq { path, text: written, expected_updated_at: cur }).is_err());
    }

    #[test]
    fn fixture_copy_under_default_folder_is_listed() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/05-running/project");
        let d = tempfile::tempdir().unwrap();
        copy(&src, &d.path().join("work/errands"));
        let l = list(&FabricListReq { root: d.path().to_string_lossy().into_owned(), home: None }).unwrap();
        assert_eq!(l.records.len(), 1);
        assert!(l.records[0].path.ends_with("work/errands/.fabric-space/tasks/2026-10-24-order-filament.md"));
    }

    #[test]
    fn create_derives_id_and_collides_to_suffix() {
        let d = tempfile::tempdir().unwrap();
        let req = FabricCreateReq {
            folder: d.path().to_string_lossy().into_owned(),
            title: "  Order 2 kg of Black PLA!! ".into(),
            due: Some("2026-10-28".into()),
            pri: None,
            context: Some("Same brand as last time.".into()),
            spec: None,
            success: Some("it arrives".into()),
        };
        let a = create_in(&req, d.path()).unwrap();
        let b = create_in(&req, d.path()).unwrap();
        assert!(a.id.ends_with("-order-2-kg-of-black-pla") && a.id.len() == 11 + 23, "{}", a.id);
        assert_eq!(b.id, format!("{}-2", a.id));
        assert!(a.path.ends_with(&format!(".fabric-space/tasks/{}.md", a.id)));
        let long = FabricCreateReq { title: "x".repeat(80), ..req.clone() };
        assert_eq!(create_in(&long, d.path()).unwrap().id.len(), 11 + 40);
        let left: Vec<_> = std::fs::read_dir(d.path().join(".fabric-space/tasks")).unwrap().flatten().collect();
        assert_eq!(left.len(), 3);
    }

    #[test]
    fn create_writes_the_minimal_record() {
        let d = tempfile::tempdir().unwrap();
        let req = FabricCreateReq {
            folder: d.path().to_string_lossy().into_owned(),
            title: "order: \"filament\"".into(),
            due: None,
            pri: None,
            context: Some("ctx\n## Plan\nmore".into()),
            spec: Some("do it".into()),
            success: None,
        };
        let r = create_in(&req, d.path()).unwrap();
        let text = read(&FabricReadReq { path: r.path.clone() }).unwrap().text;
        assert!(text.starts_with("---\nschema: 7\n"));
        assert_eq!(updated_at(&text).len(), 20);
        assert!(text.contains(&format!("id: {}\n", r.id)));
        assert!(text.contains("title: \"order: \\\"filament\\\"\"\ndue: null\npri: 0\nstate: reminder\nstep: null\nwaiting: false\n"));
        assert!(
            text.contains("created_at: ")
                && text.contains(
                    "requests: {delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null}\n"
                )
        );
        assert!(text.contains(
            "grants: {requested: [], granted: []}\nplan_meta: null\nassertions: []\napproval: null\nverdict: null\ncost_usd: 0\n"
        ));
        assert!(text.ends_with("---\n## Context\nctx\n ## Plan\nmore\n## Spec\ndo it\n## Success\n## Plan\n## Log\n"));
        let listed = list_in(&FabricListReq { root: req.folder.clone(), home: None }, d.path()).unwrap();
        assert_eq!(listed.records.len(), 1);
    }

    #[test]
    fn create_refuses_bad_requests() {
        let d = tempfile::tempdir().unwrap();
        let ok = FabricCreateReq {
            folder: d.path().to_string_lossy().into_owned(),
            title: "a".into(),
            due: None,
            pri: None,
            context: None,
            spec: None,
            success: None,
        };
        for title in ["", "   "] {
            let r = create_in(&FabricCreateReq { title: title.into(), ..ok.clone() }, d.path());
            assert!(matches!(r, Err(FabrError::BadRequest(_))));
        }
        let missing = FabricCreateReq { folder: d.path().join("nope").to_string_lossy().into_owned(), ..ok.clone() };
        assert!(matches!(create_in(&missing, d.path()), Err(FabrError::NotFound(_))));
        let bad_due = FabricCreateReq { due: Some("tomorrow".into()), ..ok.clone() };
        assert!(matches!(create_in(&bad_due, d.path()), Err(FabrError::BadRequest(_))));
        assert!(!d.path().join(".fabric-space").exists());
        let home = FabricCreateReq { folder: "~".into(), ..ok };
        assert!(create_in(&home, d.path()).is_ok());
    }

    #[test]
    fn slugs_follow_the_spec() {
        assert_eq!(slugify("Hello,  World"), "hello-world");
        assert_eq!(slugify("!!!"), "task");
        assert_eq!(slugify(&format!("{} z", "a".repeat(39))), "a".repeat(39));
    }

    fn gui_put(path: &str, rec: &TaskRecord, expected: &str) -> Result<FabricWriteRes, FabrError> {
        write(&FabricWriteReq { path: path.into(), text: taskrec::render(rec), expected_updated_at: expected.into() })
    }

    #[test]
    fn the_gui_edits_sections_until_execution_starts_and_not_after() {
        let d = tempfile::tempdir().unwrap();
        let made = create_in(&FabricCreateReq { spec: Some("old spec".into()), ..create_req(d.path(), "edit me") }, d.path()).unwrap();
        let read_rec = || taskrec::parse(&read(&FabricReadReq { path: made.path.clone() }).unwrap().text).unwrap();
        let mut rec = read_rec();
        rec.spec = "new spec".into();
        rec.title = "renamed".into();
        gui_put(&made.path, &rec, &rec.updated_at.clone()).unwrap();
        let after = read_rec();
        assert_eq!((after.spec.as_str(), after.title.as_str()), ("new spec", "renamed"));
        assert_eq!((after.by.spec, after.by.plan, after.by.context), (Some(taskrec::Author::Human), None, None));
        let mut rec = after;
        rec.state = State::Defined;
        rec.step = Some(Step::Approve);
        rec.plan = "1. do it".into();
        rec.approval = Some(taskrec::Approval { status: "approved".into(), by: Some("gui".into()), at: None, note: None, digest: None });
        rec.approval.as_mut().unwrap().digest = Some(taskrec::approval_digest(&rec));
        taskrec::write_record(&PathBuf::from(&made.path), &rec, None).unwrap();
        let mut rec = read_rec();
        rec.success = "changed success".into();
        gui_put(&made.path, &rec, &rec.updated_at.clone()).unwrap();
        let after = read_rec();
        assert_eq!(after.requests.redo, Some(taskrec::RedoStage::Plan));
        assert_eq!(after.approval.as_ref().map(|a| a.status.as_str()), Some("pending"));
        assert_eq!(after.approval.as_ref().unwrap().digest, Some(taskrec::approval_digest(&after)));
        let mut rec = after;
        rec.state = State::InProgress;
        rec.step = Some(Step::Execute);
        taskrec::write_record(&PathBuf::from(&made.path), &rec, None).unwrap();
        let mut rec = read_rec();
        rec.plan = "sneaky".into();
        let err = gui_put(&made.path, &rec, &rec.updated_at.clone()).unwrap_err();
        assert!(matches!(&err, FabrError::BadRequest(m) if m == "task is frozen once execution starts"), "{err:?}");
        let mut rec = read_rec();
        rec.requests.pause = true;
        rec.suspended = true;
        gui_put(&made.path, &rec, &rec.updated_at.clone()).unwrap();
        assert!(read_rec().requests.pause);
    }

    fn create_req(dir: &Path, title: &str) -> FabricCreateReq {
        FabricCreateReq {
            folder: dir.to_string_lossy().into_owned(),
            title: title.into(),
            due: None,
            pri: None,
            context: None,
            spec: None,
            success: None,
        }
    }

    #[test]
    fn create_takes_priority_and_timed_due() {
        let d = tempfile::tempdir().unwrap();
        let req = FabricCreateReq { due: Some("2026-10-28T09:30".into()), pri: Some(3), ..create_req(d.path(), "call the bank") };
        let made = create_in(&req, d.path()).unwrap();
        let rec = taskrec::parse(&read(&FabricReadReq { path: made.path }).unwrap().text).unwrap();
        assert_eq!((rec.schema, rec.pri, rec.due.as_deref(), rec.state), (7, 3, Some("2026-10-28T09:30"), State::Reminder));
        for bad in ["2026-10-28T25:00", "2026-10-28T9:30", "2026-10-28 09:30"] {
            let r = create_in(&FabricCreateReq { due: Some(bad.into()), ..create_req(d.path(), "x") }, d.path());
            assert!(matches!(r, Err(FabrError::BadRequest(_))), "{bad}");
        }
        let r = create_in(&FabricCreateReq { pri: Some(4), ..create_req(d.path(), "x") }, d.path());
        assert!(matches!(r, Err(FabrError::BadRequest(_))));
        let items = front_matter_items(d.path());
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].pri, items[0].due.as_deref()), (3, Some("2026-10-28T09:30")));
    }

    fn written(dir: &Path, state: State) -> (String, TaskRecord) {
        let made = create_in(&create_req(dir, "pay rent"), dir).unwrap();
        let mut rec = taskrec::parse(&read(&FabricReadReq { path: made.path.clone() }).unwrap().text).unwrap();
        rec.state = state;
        (made.path, rec)
    }

    fn put_state(path: &str, rec: &TaskRecord) -> Result<FabricWriteRes, FabrError> {
        let expected = taskrec::parse(&std::fs::read_to_string(path).unwrap()).unwrap().updated_at;
        write(&FabricWriteReq { path: path.into(), text: taskrec::render(rec), expected_updated_at: expected })
    }

    #[test]
    fn a_reminder_can_be_marked_done_by_hand() {
        let d = tempfile::tempdir().unwrap();
        let (path, rec) = written(d.path(), State::Succeeded);
        put_state(&path, &rec).unwrap();
        let after = taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after.state, State::Succeeded);
        assert!(after.verdict.is_none() && after.step.is_none());
        assert!(after.log.trim_end().ends_with(" done by hand") && after.log.starts_with("- 20"), "{}", after.log);
        assert_eq!(front_matter_items(d.path())[0].phase, crate::item::Phase::Done);
    }

    #[test]
    fn every_other_state_change_is_refused() {
        let d = tempfile::tempdir().unwrap();
        for to in [State::Task, State::Defined, State::InProgress, State::Failed] {
            let (path, rec) = written(d.path(), to);
            let err = put_state(&path, &rec).unwrap_err();
            assert!(matches!(err, FabrError::BadRequest(_)), "{to:?}");
            assert_eq!(taskrec::parse(&std::fs::read_to_string(&path).unwrap()).unwrap().state, State::Reminder);
        }
        let (path, mut rec) = written(d.path(), State::Succeeded);
        put_state(&path, &rec).unwrap();
        rec.state = State::Reminder;
        assert!(put_state(&path, &rec).is_err());
        rec.state = State::Failed;
        assert!(put_state(&path, &rec).is_err());
    }

    #[test]
    fn import_creates_reminders_for_open_lines_and_renames_the_file() {
        let d = tempfile::tempdir().unwrap();
        let xs = d.path().join(".fabr");
        std::fs::create_dir_all(&xs).unwrap();
        std::fs::write(
            xs.join(".todo"),
            "# fabr todo\nwip  2026-10-19  Migrate auth  pri:2  id:3380\ndone -  Shipped\ntodo 2026-10-20T09:30  Call the bank\nnot a line\n",
        )
        .unwrap();
        let req = ImportTodosReq { path: d.path().to_string_lossy().into_owned() };
        assert_eq!(import_todos_in(&req, d.path()).unwrap().created, 2);
        assert!(!xs.join(".todo").exists() && xs.join(".todo.imported").is_file());
        let mut items = front_matter_items(d.path());
        items.sort_by(|a, b| a.title.cmp(&b.title));
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Call the bank", "Migrate auth"]);
        assert_eq!((items[0].due.as_deref(), items[0].pri), (Some("2026-10-20T09:30"), 0));
        assert_eq!((items[1].due.as_deref(), items[1].pri), (Some("2026-10-19"), 2));
        assert!(items.iter().all(|i| i.phase == crate::item::Phase::Todo));
        let text = std::fs::read_to_string(d.path().join(".fabric-space/tasks").join(format!("{}.md", items[1].id))).unwrap();
        assert!(taskrec::parse(&text).unwrap().context.contains("Imported from .fabr/.todo"));
        assert_eq!(import_todos_in(&req, d.path()).unwrap().created, 0);
        assert_eq!(front_matter_items(d.path()).len(), 2);
    }

    #[test]
    fn import_refuses_a_missing_folder() {
        let d = tempfile::tempdir().unwrap();
        let req = ImportTodosReq { path: d.path().join("nope").to_string_lossy().into_owned() };
        assert!(matches!(import_todos_in(&req, d.path()), Err(FabrError::NotFound(_))));
    }
}
