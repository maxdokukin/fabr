use crate::lantern::events::{BudgetState, RejectedAlternative, ResidualAssumption};
use crate::lantern::explore::{ExplorationState, HypothesisStatus, QuestionStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConvergenceOutcome {
    Converged,
    Expand(String),
    Escalate(String),
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvergenceVerdict {
    pub outcome: ConvergenceOutcome,
    pub stopping_rationale: String,
    pub alternatives: Vec<RejectedAlternative>,
    pub residual_assumptions: Vec<ResidualAssumption>,
    pub budget_state: BudgetState,
    pub coverage: f32,
    pub contradictions: Vec<String>,
}

/// Evaluates convergence across coverage, evidence quality, contradictions, and residual risk.
pub fn evaluate_convergence(state: &ExplorationState) -> ConvergenceVerdict {
    let total_questions = state.questions.len();
    let resolved_questions = state.questions.iter().filter(|q| q.status == QuestionStatus::Resolved).count();
    let coverage = if total_questions == 0 { 1.0 } else { resolved_questions as f32 / total_questions as f32 };

    // Gather rejected alternatives from eliminated hypotheses
    let mut alternatives = Vec::new();
    for h in &state.hypotheses {
        if let HypothesisStatus::Eliminated { reason } = &h.status {
            alternatives.push(RejectedAlternative {
                hypothesis_id: h.id.clone(),
                description: h.description.clone(),
                rejection_reason: reason.clone(),
                evidence_refs: h.opposing_evidence_ids.clone(),
            });
        }
    }

    // Residual assumptions
    let residual_assumptions: Vec<ResidualAssumption> = state
        .assumptions
        .iter()
        .map(|a| ResidualAssumption {
            assumption_id: a.id.clone(),
            text: a.text.clone(),
            risk_if_wrong: a.risk.clone(),
            could_investigate: true,
            investigation_cost: "bounded inspection".into(),
        })
        .collect();

    // Check for unresolved contradictions (stale decisions that have not been superseded)
    let mut contradictions = Vec::new();
    for dec in &state.decisions {
        if dec.is_stale {
            let has_active_resolution = state.decisions.iter().any(|d| d.question_id == dec.question_id && !d.is_stale);
            if !has_active_resolution {
                contradictions.push(format!("Decision {} became stale due to contradictory evidence", dec.id));
            }
        }
    }

    let budget_state = BudgetState {
        tool_calls_used: state.budget.used_tool_calls,
        tool_calls_remaining: state.budget.tool_calls_remaining(),
        tokens_used: state.budget.used_tokens,
        tokens_remaining: state.budget.tokens_remaining(),
        estimated_value_of_more: if state.budget.tool_calls_remaining() > 0 {
            "Moderate - additional tool calls could narrow assumptions".into()
        } else {
            "Zero - exploration budget exhausted".into()
        },
    };

    // Determine outcome
    let outcome = if !contradictions.is_empty() {
        ConvergenceOutcome::Expand(format!("Contradiction detected: {}. Reopening investigation branch.", contradictions.join("; ")))
    } else if let Some(blocking) = residual_assumptions.iter().find(|a| {
        a.risk_if_wrong.to_lowercase().contains("block")
            || a.risk_if_wrong.to_lowercase().contains("break")
            || a.risk_if_wrong.to_lowercase().contains("high")
    }) {
        ConvergenceOutcome::Escalate(format!("Unverified assumption with material risk: {} ({})", blocking.text, blocking.risk_if_wrong))
    } else if state.questions.iter().any(|q| q.status == QuestionStatus::Open) {
        if !state.budget.has_budget() {
            ConvergenceOutcome::Inconclusive
        } else {
            let next_open = state.questions.iter().find(|q| q.status == QuestionStatus::Open).unwrap();
            ConvergenceOutcome::Expand(next_open.text.clone())
        }
    } else if !state.budget.has_budget() && coverage < 1.0 {
        ConvergenceOutcome::Inconclusive
    } else {
        ConvergenceOutcome::Converged
    };

    let stopping_rationale = match &outcome {
        ConvergenceOutcome::Converged => {
            format!(
                "All {} questions investigated and resolved with {} pieces of evidence. {} alternatives evaluated and eliminated. Residual assumptions documented without critical blockers.",
                total_questions,
                state.evidence.len(),
                alternatives.len()
            )
        }
        ConvergenceOutcome::Expand(reason) => format!("Further investigation warranted: {reason}"),
        ConvergenceOutcome::Escalate(reason) => format!("Human review required before action: {reason}"),
        ConvergenceOutcome::Inconclusive => {
            format!("Exploration budget exhausted with {resolved_questions}/{total_questions} questions resolved.")
        }
    };

    ConvergenceVerdict { outcome, stopping_rationale, alternatives, residual_assumptions, budget_state, coverage, contradictions }
}
