use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Common header for every lantern event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventHeader {
    pub event_id: String,
    pub task_id: String,
    pub run_id: String,
    pub timestamp: String,       // ISO 8601 UTC
    pub node_id: String,         // position in the decision graph
    pub parent_ids: Vec<String>, // causal parents (DAG, not tree)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum LanternEvent {
    #[serde(rename = "question.opened")]
    QuestionOpened {
        #[serde(flatten)]
        header: EventHeader,
        question_id: String,
        text: String,
        context_refs: Vec<String>,
    },

    #[serde(rename = "branch.created")]
    BranchCreated {
        #[serde(flatten)]
        header: EventHeader,
        branch_id: String,
        question_id: String,
        description: String,
    },

    #[serde(rename = "branch.prioritized")]
    BranchPrioritized {
        #[serde(flatten)]
        header: EventHeader,
        branch_id: String,
        priority: u32,
        reason: String,
    },

    #[serde(rename = "evidence.collected")]
    EvidenceCollected {
        #[serde(flatten)]
        header: EventHeader,
        evidence_id: String,
        source: EvidenceSource,
        summary: String,
        content_hash: String,
    },

    #[serde(rename = "evidence.contradicts")]
    EvidenceContradicts {
        #[serde(flatten)]
        header: EventHeader,
        evidence_id: String,
        contradicts_decision_id: String,
        explanation: String,
    },

    #[serde(rename = "assumption.declared")]
    AssumptionDeclared {
        #[serde(flatten)]
        header: EventHeader,
        assumption_id: String,
        text: String,
        basis: String,
        risk: String,
    },

    #[serde(rename = "hypothesis.formed")]
    HypothesisFormed {
        #[serde(flatten)]
        header: EventHeader,
        hypothesis_id: String,
        description: String,
        evidence_refs: Vec<String>,
    },

    #[serde(rename = "hypothesis.eliminated")]
    HypothesisEliminated {
        #[serde(flatten)]
        header: EventHeader,
        hypothesis_id: String,
        reason: String,
        evidence_refs: Vec<String>,
    },

    #[serde(rename = "branch.pruned")]
    BranchPruned {
        #[serde(flatten)]
        header: EventHeader,
        branch_id: String,
        reason: String,
        by: String, // "agent" or "human"
    },

    #[serde(rename = "branch.merged")]
    BranchMerged {
        #[serde(flatten)]
        header: EventHeader,
        branch_ids: Vec<String>,
        conclusion: String,
    },

    #[serde(rename = "decision.proposed")]
    DecisionProposed {
        #[serde(flatten)]
        header: EventHeader,
        decision_id: String,
        question_id: String,
        selected_option: String,
        evidence_refs: Vec<String>,
        assumption_refs: Vec<String>,
        unresolved_questions: Vec<String>,
        stopping_rationale: String,
    },

    #[serde(rename = "decision.stale")]
    DecisionStale {
        #[serde(flatten)]
        header: EventHeader,
        decision_id: String,
        cause: String, // the event_id or steering action that caused invalidation
    },

    #[serde(rename = "gate.evaluated")]
    GateEvaluated {
        #[serde(flatten)]
        header: EventHeader,
        gate_id: String,
        decision_id: String,
        verdict: GateVerdict,
    },

    #[serde(rename = "gate.overridden")]
    GateOverridden {
        #[serde(flatten)]
        header: EventHeader,
        gate_id: String,
        by: String,
        reason: String,
    },

    #[serde(rename = "action.authorized")]
    ActionAuthorized {
        #[serde(flatten)]
        header: EventHeader,
        decision_id: String,
        by: String,
        at: String,
    },

    #[serde(rename = "action.executed")]
    ActionExecuted {
        #[serde(flatten)]
        header: EventHeader,
        action_id: String,
        tool_name: String,
        tool_params: serde_json::Value,
    },

    #[serde(rename = "action.effect")]
    ActionEffect {
        #[serde(flatten)]
        header: EventHeader,
        action_id: String,
        effect_type: String, // "file_write", "file_create", "api_call", etc.
        target: String,
        reversible: bool,
        reversal: Option<serde_json::Value>,
    },

    #[serde(rename = "outcome.recorded")]
    OutcomeRecorded {
        #[serde(flatten)]
        header: EventHeader,
        outcome_id: String,
        decision_id: String,
        result: String,
        passed: bool,
    },

    #[serde(rename = "steering.applied")]
    SteeringApplied {
        #[serde(flatten)]
        header: EventHeader,
        steering_id: String,
        action: String, // "explore_further", "reject_assumption", etc.
        target_node: String,
        by: String,
        detail: String,
    },
}

impl LanternEvent {
    pub fn header(&self) -> &EventHeader {
        match self {
            LanternEvent::QuestionOpened { header, .. }
            | LanternEvent::BranchCreated { header, .. }
            | LanternEvent::BranchPrioritized { header, .. }
            | LanternEvent::EvidenceCollected { header, .. }
            | LanternEvent::EvidenceContradicts { header, .. }
            | LanternEvent::AssumptionDeclared { header, .. }
            | LanternEvent::HypothesisFormed { header, .. }
            | LanternEvent::HypothesisEliminated { header, .. }
            | LanternEvent::BranchPruned { header, .. }
            | LanternEvent::BranchMerged { header, .. }
            | LanternEvent::DecisionProposed { header, .. }
            | LanternEvent::DecisionStale { header, .. }
            | LanternEvent::GateEvaluated { header, .. }
            | LanternEvent::GateOverridden { header, .. }
            | LanternEvent::ActionAuthorized { header, .. }
            | LanternEvent::ActionExecuted { header, .. }
            | LanternEvent::ActionEffect { header, .. }
            | LanternEvent::OutcomeRecorded { header, .. }
            | LanternEvent::SteeringApplied { header, .. } => header,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    File { path: String },
    ToolCall { tool: String, params: serde_json::Value },
    Search { query: String },
    UserInput { question_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateVerdict {
    pub outcome: GateOutcome,
    pub stopping_rationale: String,
    pub alternatives: Vec<RejectedAlternative>,
    pub residual_assumptions: Vec<ResidualAssumption>,
    pub budget_state: BudgetState,
    pub coverage: f32,
    pub contradictions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    Converged,
    Expand,
    Escalate,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedAlternative {
    pub hypothesis_id: String,
    pub description: String,
    pub rejection_reason: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResidualAssumption {
    pub assumption_id: String,
    pub text: String,
    pub risk_if_wrong: String,
    pub could_investigate: bool,
    pub investigation_cost: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetState {
    pub tool_calls_used: u32,
    pub tool_calls_remaining: u32,
    pub tokens_used: u64,
    pub tokens_remaining: u64,
    pub estimated_value_of_more: String,
}

/// Manages the append-only event log for a single run.
pub struct RunLog {
    dir: PathBuf,
    run_id: String,
}

impl RunLog {
    /// Create a new run log. Creates the directory structure if needed.
    pub fn new(fabric_space: &Path, task_id: &str, run_id: &str) -> Result<Self, fabr_core::FabrError> {
        let dir = fabric_space.join("runs").join(task_id).join(run_id);
        fs::create_dir_all(&dir).map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", dir.display())))?;
        Ok(Self { dir, run_id: run_id.to_string() })
    }

    /// Append an event to events.jsonl.
    pub fn append(&self, event: &LanternEvent) -> Result<(), fabr_core::FabrError> {
        let path = self.dir.join("events.jsonl");
        let mut line = serde_json::to_string(event).map_err(|e| fabr_core::FabrError::BadRequest(format!("event serialize: {e}")))?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", path.display())))?;
        file.write_all(line.as_bytes()).map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", path.display())))?;
        Ok(())
    }

    /// Read all events from the log.
    pub fn read_all(&self) -> Result<Vec<LanternEvent>, fabr_core::FabrError> {
        let path = self.dir.join("events.jsonl");
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut events = Vec::new();
                for (i, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let event: LanternEvent = serde_json::from_str(line)
                        .map_err(|e| fabr_core::FabrError::BadRequest(format!("events.jsonl line {}: {e}", i + 1)))?;
                    events.push(event);
                }
                Ok(events)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(fabr_core::FabrError::Io(format!("{}: {e}", path.display()))),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Generate a unique run ID: ISO timestamp + 8 random hex chars.
pub fn new_run_id() -> String {
    let ts = Utc::now().format("%Y%m%dT%H%M%S");
    let hex: String = (0..4).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
    format!("{ts}-{hex}")
}

/// Generate a unique event ID.
pub fn new_event_id() -> String {
    let hex: String = (0..8).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
    format!("ev-{hex}")
}

/// Generate a unique node ID.
pub fn new_node_id() -> String {
    let hex: String = (0..6).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
    format!("n-{hex}")
}

/// Helper to create a header with common fields.
pub fn header(task_id: &str, run_id: &str, parent_ids: Vec<String>) -> EventHeader {
    EventHeader {
        event_id: new_event_id(),
        task_id: task_id.to_string(),
        run_id: run_id.to_string(),
        timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        node_id: new_node_id(),
        parent_ids,
    }
}

/// A single entry in the run index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunIndexEntry {
    pub run_id: String,
    pub task_id: String,
    pub task_path: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: RunStatus,
    pub event_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Active,
    Converged,
    Escalated,
    Inconclusive,
    Completed,
    Failed,
}

/// Append a run entry to the task's run index.
pub fn index_run(fabric_space: &Path, entry: &RunIndexEntry) -> Result<(), fabr_core::FabrError> {
    let path = fabric_space.join("runs").join(&entry.task_id).join("index.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", parent.display())))?;
    }
    let mut line = serde_json::to_string(entry).map_err(|e| fabr_core::FabrError::BadRequest(format!("index serialize: {e}")))?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", path.display())))?;
    file.write_all(line.as_bytes()).map_err(|e| fabr_core::FabrError::Io(format!("{}: {e}", path.display())))?;
    Ok(())
}

/// List all runs for a task.
pub fn list_runs(fabric_space: &Path, task_id: &str) -> Result<Vec<RunIndexEntry>, fabr_core::FabrError> {
    let path = fabric_space.join("runs").join(task_id).join("index.jsonl");
    match fs::read_to_string(&path) {
        Ok(text) => {
            let mut entries = Vec::new();
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str(line) {
                    entries.push(entry);
                }
            }
            Ok(entries)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(fabr_core::FabrError::Io(format!("{}: {e}", path.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_log_write_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let log = RunLog::new(tmp.path(), "test-task", "run-001").unwrap();

        let h = header("test-task", "run-001", vec![]);
        let event = LanternEvent::QuestionOpened {
            header: h,
            question_id: "q1".into(),
            text: "What auth system exists?".into(),
            context_refs: vec!["gathered".into()],
        };
        log.append(&event).unwrap();

        let events = log.read_all().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            LanternEvent::QuestionOpened { question_id, text, .. } => {
                assert_eq!(question_id, "q1");
                assert_eq!(text, "What auth system exists?");
            }
            other => panic!("expected QuestionOpened, got {other:?}"),
        }
    }

    #[test]
    fn run_log_appends_multiple_events() {
        let tmp = tempfile::tempdir().unwrap();
        let log = RunLog::new(tmp.path(), "test-task", "run-002").unwrap();

        for i in 0..5 {
            let h = header("test-task", "run-002", vec![]);
            let event = LanternEvent::EvidenceCollected {
                header: h,
                evidence_id: format!("e{i}"),
                source: EvidenceSource::File { path: format!("file{i}.txt") },
                summary: format!("evidence {i}"),
                content_hash: format!("hash{i}"),
            };
            log.append(&event).unwrap();
        }

        let events = log.read_all().unwrap();
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn run_log_empty_on_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let log = RunLog::new(tmp.path(), "test-task", "run-003").unwrap();
        let events = log.read_all().unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn run_id_is_unique() {
        let a = new_run_id();
        let b = new_run_id();
        assert_ne!(a, b);
        assert!(a.contains('T')); // has timestamp
    }

    #[test]
    fn event_serializes_with_type_tag() {
        let h = header("t", "r", vec![]);
        let event = LanternEvent::GateEvaluated {
            header: h,
            gate_id: "g1".into(),
            decision_id: "d1".into(),
            verdict: GateVerdict {
                outcome: GateOutcome::Escalate,
                stopping_rationale: "mobile compat unknown".into(),
                alternatives: vec![],
                residual_assumptions: vec![ResidualAssumption {
                    assumption_id: "a1".into(),
                    text: "mobile uses REST".into(),
                    risk_if_wrong: "mobile app breaks".into(),
                    could_investigate: true,
                    investigation_cost: "check mobile repo".into(),
                }],
                budget_state: BudgetState {
                    tool_calls_used: 5,
                    tool_calls_remaining: 15,
                    tokens_used: 2000,
                    tokens_remaining: 48000,
                    estimated_value_of_more: "high - resolves blocking question".into(),
                },
                coverage: 0.75,
                contradictions: vec![],
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"gate.evaluated\""));
        assert!(json.contains("\"outcome\":\"escalate\""));

        // Round-trip
        let parsed: LanternEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            LanternEvent::GateEvaluated { verdict, .. } => {
                assert_eq!(verdict.outcome, GateOutcome::Escalate);
                assert_eq!(verdict.residual_assumptions.len(), 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn run_index_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = RunIndexEntry {
            run_id: "run-001".into(),
            task_id: "test-task".into(),
            task_path: "/home/user/project/.fabric-space/tasks/test-task.md".into(),
            started_at: "2026-10-27T17:00:00Z".into(),
            ended_at: None,
            status: RunStatus::Active,
            event_count: 0,
        };
        index_run(tmp.path(), &entry).unwrap();
        let runs = list_runs(tmp.path(), "test-task").unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-001");
        assert_eq!(runs[0].status, RunStatus::Active);
    }
}
