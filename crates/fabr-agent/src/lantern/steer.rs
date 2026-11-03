use crate::lantern::events::{LanternEvent, RunLog, header};
use crate::lantern::explore::{BranchStatus, ExplorationState, QuestionStatus};
use crate::lantern::graph::{LanternGraph, NodeStatus};
use chrono::Utc;
use fabr_core::FabrError;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SideEffect {
    pub event_id: String,
    pub action_id: String,
    pub effect_type: String, // e.g. "file_write", "file_create", "api_call"
    pub target: String,
    pub reversible: bool,
    pub reversal: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct EffectsLog {
    path: PathBuf,
}

impl EffectsLog {
    pub fn new(run_dir: &Path) -> Self {
        Self { path: run_dir.join("effects.jsonl") }
    }

    pub fn record(&self, effect: &SideEffect) -> Result<(), FabrError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| FabrError::Io(format!("{}: {e}", self.path.display())))?;
        let line = serde_json::to_string(effect).map_err(|e| FabrError::BadRequest(format!("effects serialization: {e}")))?;
        writeln!(file, "{line}").map_err(|e| FabrError::Io(format!("{}: {e}", self.path.display())))?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<SideEffect>, FabrError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.path).map_err(|e| FabrError::Io(format!("{}: {e}", self.path.display())))?;
        let mut effects = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let effect: SideEffect = serde_json::from_str(line).map_err(|e| FabrError::BadRequest(format!("effects deserialize: {e}")))?;
            effects.push(effect);
        }
        Ok(effects)
    }

    pub fn has_irreversible_since(&self, index: usize) -> Result<bool, FabrError> {
        let all = self.read_all()?;
        Ok(all.iter().skip(index).any(|e| !e.reversible))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub run_id: String,
    pub timestamp: String,
    pub exploration_state: ExplorationState,
    pub event_index: usize,
    pub effects_count: usize,
}

impl Checkpoint {
    pub fn save(&self, run_dir: &Path) -> Result<(), FabrError> {
        let cp_dir = run_dir.join("checkpoints");
        fs::create_dir_all(&cp_dir).map_err(|e| FabrError::Io(format!("{}: {e}", cp_dir.display())))?;
        let path = cp_dir.join(format!("{}.json", self.checkpoint_id));
        let text = serde_json::to_string_pretty(self).map_err(|e| FabrError::BadRequest(format!("checkpoint serialize: {e}")))?;
        fs::write(&path, text).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;

        // Also write checkpoint.json as latest
        let latest = run_dir.join("checkpoint.json");
        let _ = fs::write(&latest, serde_json::to_string_pretty(self).unwrap_or_default());
        Ok(())
    }

    pub fn load(run_dir: &Path, checkpoint_id: &str) -> Result<Self, FabrError> {
        let path = run_dir.join("checkpoints").join(format!("{checkpoint_id}.json"));
        let text = fs::read_to_string(&path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
        serde_json::from_str(&text).map_err(|e| FabrError::BadRequest(format!("checkpoint deserialize: {e}")))
    }

    pub fn load_latest(run_dir: &Path) -> Result<Option<Self>, FabrError> {
        let path = run_dir.join("checkpoint.json");
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path).map_err(|e| FabrError::Io(format!("{}: {e}", path.display())))?;
        let cp = serde_json::from_str(&text).map_err(|e| FabrError::BadRequest(format!("checkpoint deserialize: {e}")))?;
        Ok(Some(cp))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SteeringAction {
    ExploreFurther { question_id: String },
    RejectAssumption { assumption_id: String },
    AddConstraint { text: String },
    PruneBranch { branch_id: String, reason: String },
    AuthorizeDespiteGate { gate_id: String, reason: String },
    Redirect { instructions: String },
    RewindTo { checkpoint_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteeringReport {
    pub action_type: String,
    pub affected_nodes: Vec<String>,
    pub warnings: Vec<String>,
    pub details: String,
}

/// Applies a steering action to the exploration state and records provenance events.
pub fn apply_steering(
    state: &mut ExplorationState,
    graph: &mut LanternGraph,
    run_log: &RunLog,
    effects_log: &EffectsLog,
    task_id: &str,
    action: SteeringAction,
    by: &str,
) -> Result<SteeringReport, FabrError> {
    let run_id = run_log.run_id();
    let steering_id = format!("steer-{}", Utc::now().timestamp_millis());

    match action {
        SteeringAction::ExploreFurther { question_id } => {
            let mut affected = Vec::new();
            if let Some(q) = state.questions.iter_mut().find(|q| q.id == question_id) {
                q.status = QuestionStatus::Open;
                affected.push(q.id.clone());
            } else {
                return Err(FabrError::BadRequest(format!("Question not found: {question_id}")));
            }

            if let Some(node) = graph.nodes.values_mut().find(|n| n.id == question_id) {
                node.status = NodeStatus::Active;
            }

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![question_id.clone()]),
                steering_id,
                action: "explore_further".into(),
                target_node: question_id.clone(),
                by: by.to_string(),
                detail: format!("Reopened question {question_id} for further investigation"),
            })?;

            Ok(SteeringReport {
                action_type: "explore_further".into(),
                affected_nodes: affected,
                warnings: Vec::new(),
                details: format!("Reopened question {question_id}"),
            })
        }

        SteeringAction::RejectAssumption { assumption_id } => {
            let mut affected = Vec::new();
            let mut reopened_questions = Vec::new();

            // 1. Locate assumption
            let asm_exists = state.assumptions.iter().any(|a| a.id == assumption_id);
            if !asm_exists {
                return Err(FabrError::BadRequest(format!("Assumption not found: {assumption_id}")));
            }
            affected.push(assumption_id.clone());

            // 2. Find causal descendants in graph
            let descendants = graph.causal_descendants(&assumption_id);
            for node_id in &descendants {
                if let Some(node) = graph.nodes.get_mut(node_id) {
                    node.status = NodeStatus::Stale;
                }
            }

            // 3. Mark dependent decisions in state as stale
            for dec in &mut state.decisions {
                if (dec.assumption_refs.contains(&assumption_id) || descendants.contains(&dec.id)) && !dec.is_stale {
                    dec.is_stale = true;
                    affected.push(dec.id.clone());
                    reopened_questions.push(dec.question_id.clone());

                    run_log.append(&LanternEvent::DecisionStale {
                        header: header(task_id, run_id, vec![assumption_id.clone()]),
                        decision_id: dec.id.clone(),
                        cause: format!("Rejection of assumption {assumption_id}"),
                    })?;
                }
            }

            // 4. Reopen associated questions
            for q_id in reopened_questions {
                if let Some(q) = state.questions.iter_mut().find(|q| q.id == q_id) {
                    q.status = QuestionStatus::Open;
                    affected.push(q.id.clone());
                }
            }

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![assumption_id.clone()]),
                steering_id,
                action: "reject_assumption".into(),
                target_node: assumption_id.clone(),
                by: by.to_string(),
                detail: format!("Rejected assumption {assumption_id}, invalidated dependent decisions"),
            })?;

            Ok(SteeringReport {
                action_type: "reject_assumption".into(),
                affected_nodes: affected,
                warnings: Vec::new(),
                details: format!("Rejected {assumption_id}, invalidated dependent decisions"),
            })
        }

        SteeringAction::AddConstraint { text } => {
            let q_id = state.open_question(format!("Satisfy constraint: {text}"), None);
            run_log.append(&LanternEvent::QuestionOpened {
                header: header(task_id, run_id, vec![]),
                question_id: q_id.clone(),
                text: format!("Satisfy constraint: {text}"),
                context_refs: vec![],
            })?;

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![q_id.clone()]),
                steering_id,
                action: "add_constraint".into(),
                target_node: q_id.clone(),
                by: by.to_string(),
                detail: format!("Added user constraint: {text}"),
            })?;

            Ok(SteeringReport {
                action_type: "add_constraint".into(),
                affected_nodes: vec![q_id],
                warnings: Vec::new(),
                details: format!("Constraint added: {text}"),
            })
        }

        SteeringAction::PruneBranch { branch_id, reason } => {
            let mut affected = Vec::new();
            if let Some(b) = state.branches.iter_mut().find(|b| b.id == branch_id) {
                b.status = BranchStatus::Pruned;
                affected.push(b.id.clone());
            } else {
                return Err(FabrError::BadRequest(format!("Branch not found: {branch_id}")));
            }

            if let Some(node) = graph.nodes.values_mut().find(|n| n.id == branch_id) {
                node.status = NodeStatus::Pruned;
            }

            run_log.append(&LanternEvent::BranchPruned {
                header: header(task_id, run_id, vec![branch_id.clone()]),
                branch_id: branch_id.clone(),
                reason: reason.clone(),
                by: by.to_string(),
            })?;

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![branch_id.clone()]),
                steering_id,
                action: "prune_branch".into(),
                target_node: branch_id.clone(),
                by: by.to_string(),
                detail: format!("Pruned branch {branch_id}: {reason}"),
            })?;

            Ok(SteeringReport {
                action_type: "prune_branch".into(),
                affected_nodes: affected,
                warnings: Vec::new(),
                details: format!("Pruned branch {branch_id}: {reason}"),
            })
        }

        SteeringAction::AuthorizeDespiteGate { gate_id, reason } => {
            run_log.append(&LanternEvent::GateOverridden {
                header: header(task_id, run_id, vec![gate_id.clone()]),
                gate_id: gate_id.clone(),
                by: by.to_string(),
                reason: reason.clone(),
            })?;

            let dec_id = state.decisions.last().map(|d| d.id.clone()).unwrap_or_else(|| "dec-1".into());
            run_log.append(&LanternEvent::ActionAuthorized {
                header: header(task_id, run_id, vec![gate_id.clone()]),
                decision_id: dec_id.clone(),
                by: by.to_string(),
                at: Utc::now().to_rfc3339(),
            })?;

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![gate_id.clone()]),
                steering_id,
                action: "authorize_despite_gate".into(),
                target_node: gate_id.clone(),
                by: by.to_string(),
                detail: format!("Authorized despite gate {gate_id}: {reason}"),
            })?;

            Ok(SteeringReport {
                action_type: "authorize_despite_gate".into(),
                affected_nodes: vec![gate_id, dec_id],
                warnings: vec!["Human risk acceptance granted: proceeding despite gate warnings".into()],
                details: format!("Authorized execution despite gate failures: {reason}"),
            })
        }

        SteeringAction::Redirect { instructions } => {
            let mut warnings = Vec::new();
            let all_effects = effects_log.read_all().unwrap_or_default();
            if !all_effects.is_empty() {
                warnings.push(format!(
                    "Redirect issued after {} real-world actions were executed. Unreversed side-effects exist.",
                    all_effects.len()
                ));
            }

            let q_id = state.open_question(format!("Redirect: {instructions}"), None);
            run_log.append(&LanternEvent::QuestionOpened {
                header: header(task_id, run_id, vec![]),
                question_id: q_id.clone(),
                text: format!("Redirect: {instructions}"),
                context_refs: vec![],
            })?;

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![q_id.clone()]),
                steering_id,
                action: "redirect".into(),
                target_node: q_id.clone(),
                by: by.to_string(),
                detail: format!("Redirected exploration: {instructions}"),
            })?;

            Ok(SteeringReport {
                action_type: "redirect".into(),
                affected_nodes: vec![q_id],
                warnings,
                details: format!("Redirected to: {instructions}"),
            })
        }

        SteeringAction::RewindTo { checkpoint_id } => {
            let run_dir = run_log.dir();
            let cp = Checkpoint::load(run_dir, &checkpoint_id)?;
            let mut warnings = Vec::new();

            let all_effects = effects_log.read_all().unwrap_or_default();
            let effects_after = all_effects.iter().skip(cp.effects_count).collect::<Vec<_>>();
            for eff in &effects_after {
                if !eff.reversible {
                    warnings.push(format!("Irreversible side effect detected: {} on {}", eff.effect_type, eff.target));
                } else {
                    warnings.push(format!("Reversible effect: {} on {} (reversal available)", eff.effect_type, eff.target));
                }
            }

            *state = cp.exploration_state;

            run_log.append(&LanternEvent::SteeringApplied {
                header: header(task_id, run_id, vec![]),
                steering_id,
                action: "rewind_to".into(),
                target_node: checkpoint_id.clone(),
                by: by.to_string(),
                detail: format!("Rewound state to checkpoint {checkpoint_id}"),
            })?;

            Ok(SteeringReport {
                action_type: "rewind_to".into(),
                affected_nodes: vec![checkpoint_id.clone()],
                warnings,
                details: format!("Restored state to checkpoint {checkpoint_id}"),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lantern::events::{EvidenceSource, RunLog};
    use crate::lantern::explore::ExplorationBudget;
    use tempfile::tempdir;

    #[test]
    fn reject_assumption_invalidates_descendants() {
        let dir = tempdir().unwrap();
        let log = RunLog::new(dir.path(), "task-1", "run-1").unwrap();
        let effects = EffectsLog::new(log.dir());

        let mut state = ExplorationState::new(ExplorationBudget::default());
        let q1 = state.open_question("Q1", None);
        let b1 = state.open_branch(&q1, "B1", 0);
        let ev1 = state.add_evidence(&b1, EvidenceSource::File { path: "f".into() }, "ev1", "data");
        let h1 = state.add_hypothesis(&q1, "H1", vec![ev1.clone()]);
        let asm1 = state.declare_assumption("Assumption 1", "intuition", "critical");
        let dec1 = state.propose_decision(&q1, &h1, "Option 1", vec![ev1], vec![asm1.clone()], "reason");

        let mut graph = LanternGraph::new();
        // Add edge from asm1 to dec1
        graph.edges.push(crate::lantern::graph::LanternEdge {
            from: asm1.clone(),
            to: dec1.clone(),
            kind: crate::lantern::graph::EdgeKind::CausalParent,
        });

        assert_eq!(state.questions[0].status, QuestionStatus::Resolved);
        assert!(!state.decisions[0].is_stale);

        let report = apply_steering(
            &mut state,
            &mut graph,
            &log,
            &effects,
            "task-1",
            SteeringAction::RejectAssumption { assumption_id: asm1.clone() },
            "user",
        )
        .unwrap();

        assert!(report.affected_nodes.contains(&asm1));
        assert!(report.affected_nodes.contains(&dec1));
        assert!(state.decisions[0].is_stale);
        assert_eq!(state.questions[0].status, QuestionStatus::Open);
    }

    #[test]
    fn checkpoint_save_and_rewind() {
        let dir = tempdir().unwrap();
        let log = RunLog::new(dir.path(), "task-1", "run-1").unwrap();
        let effects = EffectsLog::new(log.dir());

        let mut state = ExplorationState::new(ExplorationBudget::default());
        let _q1 = state.open_question("Q1", None);

        let cp = Checkpoint {
            checkpoint_id: "cp-1".into(),
            run_id: "run-1".into(),
            timestamp: Utc::now().to_rfc3339(),
            exploration_state: state.clone(),
            event_index: 1,
            effects_count: 0,
        };
        cp.save(log.dir()).unwrap();

        // Advance state
        let _q2 = state.open_question("Q2", None);
        assert_eq!(state.questions.len(), 2);

        // Record irreversible effect
        effects
            .record(&SideEffect {
                event_id: "e1".into(),
                action_id: "a1".into(),
                effect_type: "deploy".into(),
                target: "prod".into(),
                reversible: false,
                reversal: None,
            })
            .unwrap();

        let mut graph = LanternGraph::new();
        let report = apply_steering(
            &mut state,
            &mut graph,
            &log,
            &effects,
            "task-1",
            SteeringAction::RewindTo { checkpoint_id: "cp-1".into() },
            "user",
        )
        .unwrap();

        assert_eq!(state.questions.len(), 1);
        assert!(report.warnings.iter().any(|w| w.contains("Irreversible side effect")));
    }
}
