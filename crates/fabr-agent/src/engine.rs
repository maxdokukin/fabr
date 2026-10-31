use crate::verify::CheckOutcome;
use fabr_core::FabrError;
use fabr_core::taskrec::Assertion;
use serde::Deserialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DraftAssertion {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DraftCheck {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DefineOut {
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub success: String,
    #[serde(default)]
    pub assertions: Vec<DraftAssertion>,
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PlanOut {
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub reversibility: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub checks: Vec<DraftCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecStatus {
    Done,
    Ask,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExecuteOut {
    pub status: ExecStatus,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub question: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Judgement {
    Pass,
    Fail,
    Unchecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct JudgedItem {
    pub id: String,
    pub verdict: Judgement,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct EvaluateOut {
    #[serde(default)]
    pub items: Vec<JudgedItem>,
}

pub struct DefineInput<'a> {
    pub cwd: &'a Path,
    pub resume: Option<&'a str>,
    pub title: &'a str,
    pub context: &'a str,
    pub answers: &'a str,
}

pub struct PlanInput<'a> {
    pub cwd: &'a Path,
    pub resume: Option<&'a str>,
    pub title: &'a str,
    pub context: &'a str,
    pub spec: &'a str,
    pub success: &'a str,
    pub assertions: &'a [Assertion],
    pub answers: &'a str,
    pub refusal_note: Option<&'a str>,
    pub rejection: Option<&'a str>,
    pub allowed_tools: &'a [String],
    pub allowed_checks: &'a [String],
}

pub struct ExecuteInput<'a> {
    pub cwd: &'a Path,
    pub resume: Option<&'a str>,
    pub title: &'a str,
    pub spec: &'a str,
    pub plan: &'a str,
    pub assertions: &'a [Assertion],
    pub answers: &'a str,
    pub allowed: &'a [String],
    pub read_only: bool,
}

pub struct EvaluateInput<'a> {
    pub cwd: &'a Path,
    pub title: &'a str,
    pub assertions: &'a [Assertion],
    pub summary: &'a str,
    pub log: &'a str,
    pub checks: &'a [CheckOutcome],
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Denial {
    pub tool: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug)]
pub enum End<T> {
    Done(T),
    Stopped,
    Failed(FabrError),
}

#[derive(Debug)]
pub struct Run<T> {
    pub session_id: Option<String>,
    pub cost_usd: f64,
    pub usage: Usage,
    pub denials: Vec<Denial>,
    pub end: End<T>,
}

impl<T> Run<T> {
    pub fn failed(err: FabrError) -> Run<T> {
        Run { session_id: None, cost_usd: 0.0, usage: Usage::default(), denials: Vec::new(), end: End::Failed(err) }
    }
}

pub trait Watch {
    fn session(&mut self, _id: &str) {}
    fn tool(&mut self, _tool: &ToolUse) {}
    fn stop(&mut self) -> bool {
        false
    }
}

pub struct NoWatch;

impl Watch for NoWatch {}

pub trait Engine {
    fn define(&self, input: &DefineInput, watch: &mut dyn Watch) -> Run<DefineOut>;
    fn plan(&self, input: &PlanInput, watch: &mut dyn Watch) -> Run<PlanOut>;
    fn execute(&self, input: &ExecuteInput, watch: &mut dyn Watch) -> Run<ExecuteOut>;
    fn evaluate(&self, input: &EvaluateInput, watch: &mut dyn Watch) -> Run<EvaluateOut>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptedExec {
    pub tools: Vec<ToolUse>,
    pub writes: Vec<(String, String)>,
    pub denials: Vec<Denial>,
    pub out: ExecuteOut,
}

impl ScriptedExec {
    pub fn done(summary: &str) -> ScriptedExec {
        ScriptedExec {
            tools: Vec::new(),
            writes: Vec::new(),
            denials: Vec::new(),
            out: ExecuteOut { status: ExecStatus::Done, summary: summary.into(), question: None },
        }
    }

    pub fn ask(question: &str) -> ScriptedExec {
        ScriptedExec {
            out: ExecuteOut { status: ExecStatus::Ask, summary: String::new(), question: Some(question.into()) },
            ..ScriptedExec::done("")
        }
    }

    pub fn denied(tool: &str, input: serde_json::Value) -> ScriptedExec {
        ScriptedExec {
            denials: vec![Denial { tool: tool.into(), input }],
            out: ExecuteOut { status: ExecStatus::Blocked, summary: format!("{tool} was denied"), question: None },
            ..ScriptedExec::done("")
        }
    }

    pub fn write(mut self, path: &str, content: &str) -> ScriptedExec {
        self.tools.push(ToolUse { name: "Write".into(), input: serde_json::json!({"file_path": path, "content": content}) });
        self.writes.push((path.into(), content.into()));
        self
    }

    pub fn tool(mut self, name: &str, input: serde_json::Value) -> ScriptedExec {
        self.tools.push(ToolUse { name: name.into(), input });
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    Define { resume: Option<String>, context: String, answers: String },
    Plan { resume: Option<String>, answers: String, rejection: Option<String>, refusal: Option<String> },
    Execute { resume: Option<String>, answers: String, allowed: Vec<String>, read_only: bool },
    Evaluate { summary: String, log: String },
}

#[derive(Default)]
pub struct ScriptedEngine {
    defines: Mutex<VecDeque<Result<DefineOut, String>>>,
    plans: Mutex<VecDeque<Result<PlanOut, String>>>,
    execs: Mutex<VecDeque<Result<ScriptedExec, String>>>,
    verdicts: Mutex<VecDeque<EvaluateOut>>,
    seen: Mutex<Vec<Seen>>,
    counter: Mutex<u32>,
    cost: f64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ScriptedEngine {
    pub fn new() -> ScriptedEngine {
        ScriptedEngine::default()
    }

    pub fn with_cost(mut self, usd: f64) -> ScriptedEngine {
        self.cost = usd;
        self
    }

    pub fn define(self, out: DefineOut) -> ScriptedEngine {
        lock(&self.defines).push_back(Ok(out));
        self
    }

    pub fn plan(self, out: PlanOut) -> ScriptedEngine {
        lock(&self.plans).push_back(Ok(out));
        self
    }

    pub fn exec(self, out: ScriptedExec) -> ScriptedEngine {
        lock(&self.execs).push_back(Ok(out));
        self
    }

    pub fn verdict(self, out: EvaluateOut) -> ScriptedEngine {
        lock(&self.verdicts).push_back(out);
        self
    }

    pub fn push_define(&self, out: Result<DefineOut, String>) {
        lock(&self.defines).push_back(out);
    }

    pub fn push_plan(&self, out: Result<PlanOut, String>) {
        lock(&self.plans).push_back(out);
    }

    pub fn push_exec(&self, out: Result<ScriptedExec, String>) {
        lock(&self.execs).push_back(out);
    }

    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.seen).clone()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.seen()
            .iter()
            .filter(|s| {
                matches!(
                    (kind, s),
                    ("define", Seen::Define { .. })
                        | ("plan", Seen::Plan { .. })
                        | ("execute", Seen::Execute { .. })
                        | ("evaluate", Seen::Evaluate { .. })
                )
            })
            .count()
    }

    fn session(&self, resume: Option<&str>, watch: &mut dyn Watch) -> String {
        let id = match resume {
            Some(id) => id.to_string(),
            None => {
                let mut n = lock(&self.counter);
                *n += 1;
                format!("00000000-0000-4000-8000-{:012}", *n)
            }
        };
        watch.session(&id);
        id
    }

    fn finish<T>(&self, session: String, out: Result<T, String>) -> Run<T> {
        Run {
            session_id: Some(session),
            cost_usd: self.cost,
            usage: Usage { turns: 1, ..Usage::default() },
            denials: Vec::new(),
            end: match out {
                Ok(v) => End::Done(v),
                Err(e) => End::Failed(FabrError::Remote(e)),
            },
        }
    }
}

impl Engine for ScriptedEngine {
    fn define(&self, input: &DefineInput, watch: &mut dyn Watch) -> Run<DefineOut> {
        lock(&self.seen).push(Seen::Define {
            resume: input.resume.map(str::to_string),
            context: input.context.to_string(),
            answers: input.answers.to_string(),
        });
        let session = self.session(input.resume, watch);
        let out = lock(&self.defines).pop_front().unwrap_or_else(|| Err("no scripted define".into()));
        self.finish(session, out)
    }

    fn plan(&self, input: &PlanInput, watch: &mut dyn Watch) -> Run<PlanOut> {
        lock(&self.seen).push(Seen::Plan {
            resume: input.resume.map(str::to_string),
            answers: input.answers.to_string(),
            rejection: input.rejection.map(str::to_string),
            refusal: input.refusal_note.map(str::to_string),
        });
        let session = self.session(input.resume, watch);
        let out = lock(&self.plans).pop_front().unwrap_or_else(|| Err("no scripted plan".into()));
        self.finish(session, out)
    }

    fn execute(&self, input: &ExecuteInput, watch: &mut dyn Watch) -> Run<ExecuteOut> {
        lock(&self.seen).push(Seen::Execute {
            resume: input.resume.map(str::to_string),
            answers: input.answers.to_string(),
            allowed: input.allowed.to_vec(),
            read_only: input.read_only,
        });
        let session = self.session(input.resume, watch);
        let script = match lock(&self.execs).pop_front() {
            Some(Ok(s)) => s,
            Some(Err(e)) => return self.finish(session, Err(e)),
            None => return self.finish(session, Err("no scripted execute".into())),
        };
        for tool in &script.tools {
            if watch.stop() {
                return Run { end: End::Stopped, ..self.finish(session, Ok(script.out.clone())) };
            }
            watch.tool(tool);
            if let Some((path, content)) = script.writes.iter().find(|(p, _)| tool.input["file_path"].as_str() == Some(p.as_str())) {
                let file: PathBuf = input.cwd.join(path);
                if let Some(parent) = file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&file, content);
            }
        }
        if watch.stop() {
            return Run { end: End::Stopped, ..self.finish(session, Ok(script.out.clone())) };
        }
        let mut run = self.finish(session, Ok(script.out));
        run.denials = script.denials;
        run
    }

    fn evaluate(&self, input: &EvaluateInput, watch: &mut dyn Watch) -> Run<EvaluateOut> {
        lock(&self.seen).push(Seen::Evaluate { summary: input.summary.to_string(), log: input.log.to_string() });
        let session = self.session(None, watch);
        let out = match lock(&self.verdicts).pop_front() {
            Some(v) => v,
            None => EvaluateOut {
                items: input
                    .assertions
                    .iter()
                    .map(|a| {
                        let pass = input.checks.iter().all(|c| c.pass);
                        JudgedItem {
                            id: a.id.clone(),
                            verdict: if pass { Judgement::Pass } else { Judgement::Fail },
                            evidence: if pass { "checks passed".into() } else { "a check failed".into() },
                        }
                    })
                    .collect(),
            },
        };
        self.finish(session, Ok(out))
    }
}
