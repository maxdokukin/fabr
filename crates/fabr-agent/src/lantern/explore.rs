use crate::lantern::converge::{self, ConvergenceOutcome, ConvergenceVerdict};
use crate::lantern::events::{BudgetState, EvidenceSource, GateOutcome, GateVerdict, LanternEvent, ResidualAssumption, RunLog, header};
use crate::model::ToolCall;
use crate::toolset::ToolSet;
use fabr_core::FabrError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionStatus {
    Open,
    Investigating,
    Resolved,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenQuestion {
    pub id: String,
    pub text: String,
    pub status: QuestionStatus,
    pub spawned_by_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchStatus {
    Open,
    Investigating,
    Merged,
    Pruned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub question_id: String,
    pub description: String,
    pub depth: u32,
    pub status: BranchStatus,
    pub sub_questions: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub eliminated_hypotheses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub branch_id: String,
    pub source: EvidenceSource,
    pub summary: String,
    pub content: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HypothesisStatus {
    Candidate,
    Selected,
    Eliminated { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub question_id: String,
    pub description: String,
    pub status: HypothesisStatus,
    pub supporting_evidence_ids: Vec<String>,
    pub opposing_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assumption {
    pub id: String,
    pub text: String,
    pub basis: String,
    pub risk: String,
    pub dependent_decision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedDecision {
    pub id: String,
    pub question_id: String,
    pub selected_hypothesis_id: String,
    pub selected_option: String,
    pub evidence_refs: Vec<String>,
    pub assumption_refs: Vec<String>,
    pub unresolved_questions: Vec<String>,
    pub stopping_rationale: String,
    pub is_stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationBudget {
    pub max_tool_calls: u32,
    pub max_tokens: u64,
    pub max_branches: u32,
    pub max_depth: u32,
    pub used_tool_calls: u32,
    pub used_tokens: u64,
}

impl Default for ExplorationBudget {
    fn default() -> Self {
        Self { max_tool_calls: 20, max_tokens: 50_000, max_branches: 5, max_depth: 2, used_tool_calls: 0, used_tokens: 0 }
    }
}

impl ExplorationBudget {
    pub fn has_budget(&self) -> bool {
        self.used_tool_calls < self.max_tool_calls && self.used_tokens < self.max_tokens
    }

    pub fn tool_calls_remaining(&self) -> u32 {
        self.max_tool_calls.saturating_sub(self.used_tool_calls)
    }

    pub fn tokens_remaining(&self) -> u64 {
        self.max_tokens.saturating_sub(self.used_tokens)
    }

    pub fn charge_tool_call(&mut self) {
        self.used_tool_calls = self.used_tool_calls.saturating_add(1);
    }

    pub fn charge_tokens(&mut self, tokens: u64) {
        self.used_tokens = self.used_tokens.saturating_add(tokens);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplorationState {
    pub questions: Vec<OpenQuestion>,
    pub branches: Vec<Branch>,
    pub evidence: Vec<EvidenceRecord>,
    pub hypotheses: Vec<Hypothesis>,
    pub assumptions: Vec<Assumption>,
    pub decisions: Vec<ProposedDecision>,
    pub budget: ExplorationBudget,
}

impl ExplorationState {
    pub fn new(budget: ExplorationBudget) -> Self {
        Self {
            questions: Vec::new(),
            branches: Vec::new(),
            evidence: Vec::new(),
            hypotheses: Vec::new(),
            assumptions: Vec::new(),
            decisions: Vec::new(),
            budget,
        }
    }

    pub fn open_question(&mut self, text: impl Into<String>, spawned_by_branch: Option<String>) -> String {
        let id = format!("q{}", self.questions.len() + 1);
        self.questions.push(OpenQuestion { id: id.clone(), text: text.into(), status: QuestionStatus::Open, spawned_by_branch });
        id
    }

    pub fn open_branch(&mut self, question_id: &str, description: impl Into<String>, depth: u32) -> String {
        let id = format!("b{}", self.branches.len() + 1);
        self.branches.push(Branch {
            id: id.clone(),
            question_id: question_id.to_string(),
            description: description.into(),
            depth,
            status: BranchStatus::Open,
            sub_questions: Vec::new(),
            evidence_ids: Vec::new(),
            eliminated_hypotheses: Vec::new(),
        });
        id
    }

    pub fn add_evidence(
        &mut self,
        branch_id: &str,
        source: EvidenceSource,
        summary: impl Into<String>,
        content: impl Into<String>,
    ) -> String {
        let id = format!("ev{}", self.evidence.len() + 1);
        let content_str = content.into();
        let hash = sha1_smol::Sha1::from(&content_str).digest().to_string();
        self.evidence.push(EvidenceRecord {
            id: id.clone(),
            branch_id: branch_id.to_string(),
            source,
            summary: summary.into(),
            content: content_str,
            content_hash: hash,
        });
        if let Some(b) = self.branches.iter_mut().find(|b| b.id == branch_id) {
            b.evidence_ids.push(id.clone());
        }
        id
    }

    pub fn add_hypothesis(&mut self, question_id: &str, description: impl Into<String>, supporting_evidence_ids: Vec<String>) -> String {
        let id = format!("h{}", self.hypotheses.len() + 1);
        self.hypotheses.push(Hypothesis {
            id: id.clone(),
            question_id: question_id.to_string(),
            description: description.into(),
            status: HypothesisStatus::Candidate,
            supporting_evidence_ids,
            opposing_evidence_ids: Vec::new(),
        });
        id
    }

    pub fn eliminate_hypothesis(&mut self, hypothesis_id: &str, reason: impl Into<String>, evidence_ids: Vec<String>) {
        if let Some(h) = self.hypotheses.iter_mut().find(|h| h.id == hypothesis_id) {
            h.status = HypothesisStatus::Eliminated { reason: reason.into() };
            for ev in evidence_ids {
                if !h.opposing_evidence_ids.contains(&ev) {
                    h.opposing_evidence_ids.push(ev);
                }
            }
        }
    }

    pub fn declare_assumption(&mut self, text: impl Into<String>, basis: impl Into<String>, risk: impl Into<String>) -> String {
        let id = format!("asm{}", self.assumptions.len() + 1);
        self.assumptions.push(Assumption {
            id: id.clone(),
            text: text.into(),
            basis: basis.into(),
            risk: risk.into(),
            dependent_decision_ids: Vec::new(),
        });
        id
    }

    #[allow(clippy::collapsible_if)]
    pub fn propose_decision(
        &mut self,
        question_id: &str,
        selected_hypothesis_id: &str,
        selected_option: impl Into<String>,
        evidence_refs: Vec<String>,
        assumption_refs: Vec<String>,
        stopping_rationale: impl Into<String>,
    ) -> String {
        let id = format!("dec{}", self.decisions.len() + 1);
        let unresolved: Vec<String> = self.questions.iter().filter(|q| q.status == QuestionStatus::Open).map(|q| q.id.clone()).collect();

        for asm_id in &assumption_refs {
            if let Some(asm) = self.assumptions.iter_mut().find(|a| &a.id == asm_id) {
                if !asm.dependent_decision_ids.contains(&id) {
                    asm.dependent_decision_ids.push(id.clone());
                }
            }
        }

        self.decisions.push(ProposedDecision {
            id: id.clone(),
            question_id: question_id.to_string(),
            selected_hypothesis_id: selected_hypothesis_id.to_string(),
            selected_option: selected_option.into(),
            evidence_refs,
            assumption_refs,
            unresolved_questions: unresolved,
            stopping_rationale: stopping_rationale.into(),
            is_stale: false,
        });

        if let Some(q) = self.questions.iter_mut().find(|q| q.id == question_id) {
            q.status = QuestionStatus::Resolved;
        }

        id
    }

    pub fn check_contradictions(&mut self, log: Option<&RunLog>, task_id: &str, run_id: &str) -> Vec<String> {
        let mut reopened = Vec::new();
        for dec in &mut self.decisions {
            if dec.is_stale {
                continue;
            }
            // If the selected hypothesis has opposing evidence or is eliminated
            let is_contradicted = self.hypotheses.iter().any(|h| {
                h.id == dec.selected_hypothesis_id
                    && (matches!(h.status, HypothesisStatus::Eliminated { .. }) || !h.opposing_evidence_ids.is_empty())
            });

            if is_contradicted {
                dec.is_stale = true;
                reopened.push(dec.id.clone());

                if let Some(q) = self.questions.iter_mut().find(|q| q.id == dec.question_id) {
                    q.status = QuestionStatus::Open;
                }

                if let Some(run_log) = log {
                    let _ = run_log.append(&LanternEvent::DecisionStale {
                        header: header(task_id, run_id, vec![dec.id.clone()]),
                        decision_id: dec.id.clone(),
                        cause: "contradictory evidence emerged".into(),
                    });
                }
            }
        }
        reopened
    }
}

#[derive(Debug, Clone)]
pub enum ExplorationResult {
    Converged(ConvergenceVerdict),
    NeedsHuman(String, ResidualAssumption),
    Inconclusive(String),
}

/// The Exploration Controller executes the dynamic exploration loop.
pub fn run_exploration_loop(
    state: &mut ExplorationState,
    task_id: &str,
    node_dir: &Path,
    tools: &dyn ToolSet,
    run_log: &RunLog,
) -> Result<ExplorationResult, FabrError> {
    let run_id = run_log.run_id();

    while state.budget.has_budget() {
        // 1. Identify open questions
        let open_count = state.questions.iter().filter(|q| q.status == QuestionStatus::Open).count();
        if open_count == 0 && !state.hypotheses.is_empty() {
            // Check convergence
            let verdict = converge::evaluate_convergence(state);
            run_log.append(&LanternEvent::GateEvaluated {
                header: header(task_id, run_id, vec![]),
                gate_id: format!("gate-{}", state.decisions.len()),
                decision_id: state.decisions.last().map(|d| d.id.clone()).unwrap_or_default(),
                verdict: GateVerdict {
                    outcome: match verdict.outcome {
                        ConvergenceOutcome::Converged => GateOutcome::Converged,
                        ConvergenceOutcome::Expand(_) => GateOutcome::Expand,
                        ConvergenceOutcome::Escalate(_) => GateOutcome::Escalate,
                        ConvergenceOutcome::Inconclusive => GateOutcome::Inconclusive,
                    },
                    stopping_rationale: verdict.stopping_rationale.clone(),
                    alternatives: verdict.alternatives.clone(),
                    residual_assumptions: verdict.residual_assumptions.clone(),
                    budget_state: BudgetState {
                        tool_calls_used: state.budget.used_tool_calls,
                        tool_calls_remaining: state.budget.tool_calls_remaining(),
                        tokens_used: state.budget.used_tokens,
                        tokens_remaining: state.budget.tokens_remaining(),
                        estimated_value_of_more: verdict.budget_state.estimated_value_of_more.clone(),
                    },
                    coverage: verdict.coverage,
                    contradictions: verdict.contradictions.clone(),
                },
            })?;

            match verdict.outcome {
                ConvergenceOutcome::Converged => return Ok(ExplorationResult::Converged(verdict)),
                ConvergenceOutcome::Expand(q_text) => {
                    let q_id = state.open_question(q_text.clone(), None);
                    run_log.append(&LanternEvent::QuestionOpened {
                        header: header(task_id, run_id, vec![]),
                        question_id: q_id,
                        text: q_text,
                        context_refs: vec![],
                    })?;
                    continue;
                }
                ConvergenceOutcome::Escalate(reason) => {
                    let fallback_asm = ResidualAssumption {
                        assumption_id: "asm-unresolved".into(),
                        text: reason.clone(),
                        risk_if_wrong: "Decision may fail unexpectedly".into(),
                        could_investigate: true,
                        investigation_cost: "Human confirmation required".into(),
                    };
                    let asm = verdict.residual_assumptions.first().cloned().unwrap_or(fallback_asm);
                    return Ok(ExplorationResult::NeedsHuman(reason, asm));
                }
                ConvergenceOutcome::Inconclusive => {
                    return Ok(ExplorationResult::Inconclusive(verdict.stopping_rationale));
                }
            }
        }

        // If no open questions and no hypotheses, nothing to investigate
        if open_count == 0 {
            break;
        }

        // 2. Pick next open question
        let q_idx = state.questions.iter().position(|q| q.status == QuestionStatus::Open).unwrap();
        let question = state.questions[q_idx].clone();
        state.questions[q_idx].status = QuestionStatus::Investigating;

        // 3. Open a branch
        let branch_id = state.open_branch(&question.id, format!("Investigate {}", question.text), 0);
        run_log.append(&LanternEvent::BranchCreated {
            header: header(task_id, run_id, vec![question.id.clone()]),
            branch_id: branch_id.clone(),
            question_id: question.id.clone(),
            description: format!("Investigate {}", question.text),
        })?;

        // 4. Gather evidence using available read-only tools
        if tools.names().contains(&"list_dir".to_string()) && state.budget.has_budget() {
            state.budget.charge_tool_call();
            let call = ToolCall { name: "list_dir".into(), path: ".".into(), content: None, params: None };
            if let Ok(files) = tools.call(node_dir, &call) {
                let ev_id = state.add_evidence(&branch_id, EvidenceSource::File { path: ".".into() }, "Directory listing", &files);
                run_log.append(&LanternEvent::EvidenceCollected {
                    header: header(task_id, run_id, vec![branch_id.clone()]),
                    evidence_id: ev_id,
                    source: EvidenceSource::File { path: ".".into() },
                    summary: "Directory listing".into(),
                    content_hash: sha1_smol::Sha1::from(&files).digest().to_string(),
                })?;
            }
        }

        // 5. Evaluate hypotheses & check contradictions
        state.check_contradictions(Some(run_log), task_id, run_id);
    }

    if !state.budget.has_budget() {
        let verdict = converge::evaluate_convergence(state);
        Ok(ExplorationResult::Inconclusive(format!("Exploration budget exhausted: {}", verdict.stopping_rationale)))
    } else {
        let verdict = converge::evaluate_convergence(state);
        Ok(ExplorationResult::Converged(verdict))
    }
}
