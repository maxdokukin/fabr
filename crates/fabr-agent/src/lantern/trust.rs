use crate::lantern::converge::{ConvergenceOutcome, ConvergenceVerdict};
use crate::lantern::explore::{ExplorationBudget, ExplorationState};
use fabr_core::FabrError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoContinuePolicy {
    #[serde(default = "default_true")]
    pub read_only_investigation: bool,
    #[serde(default = "default_true")]
    pub safe_branch_elimination: bool,
    #[serde(default = "default_within_approved_scope")]
    pub reversible_file_changes: String,
}

impl Default for AutoContinuePolicy {
    fn default() -> Self {
        Self { read_only_investigation: true, safe_branch_elimination: true, reversible_file_changes: "within_approved_scope".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequireReviewPolicy {
    #[serde(default = "default_false")]
    pub unverified_assumptions: bool,
    #[serde(default = "default_always")]
    pub irreversible_actions: String,
    #[serde(default = "default_true")]
    pub contradictory_evidence: bool,
    #[serde(default = "default_true")]
    pub choosing_between_equal: bool,
}

impl Default for RequireReviewPolicy {
    fn default() -> Self {
        Self {
            unverified_assumptions: false,
            irreversible_actions: "always".into(),
            contradictory_evidence: true,
            choosing_between_equal: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls_per_exploration: u32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens_per_exploration: u64,
    #[serde(default = "default_max_branches")]
    pub max_branches: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self { max_tool_calls_per_exploration: 20, max_tokens_per_exploration: 50_000, max_branches: 5, max_depth: 2 }
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_always() -> String {
    "always".into()
}

fn default_within_approved_scope() -> String {
    "within_approved_scope".into()
}

fn default_max_tool_calls() -> u32 {
    20
}

fn default_max_tokens() -> u64 {
    50_000
}

fn default_max_branches() -> u32 {
    5
}

fn default_max_depth() -> u32 {
    2
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutonomyPolicy {
    #[serde(default)]
    pub auto_continue: AutoContinuePolicy,
    #[serde(default)]
    pub require_review: RequireReviewPolicy,
    #[serde(default)]
    pub budget: BudgetPolicy,
}

impl AutonomyPolicy {
    pub fn load_or_default(fabric_space: &Path) -> Self {
        let path = fabric_space.join("autonomy.toml");
        if let Ok(text) = std::fs::read_to_string(&path) { Self::parse_toml(&text).unwrap_or_default() } else { Self::default() }
    }

    pub fn parse_toml(text: &str) -> Result<Self, FabrError> {
        let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| FabrError::BadRequest(format!("autonomy.toml: {e}")))?;
        let mut policy = Self::default();

        if let Some(ac) = doc.get("auto_continue") {
            if let Some(v) = ac.get("read_only_investigation").and_then(|v| v.as_bool()) {
                policy.auto_continue.read_only_investigation = v;
            }
            if let Some(v) = ac.get("safe_branch_elimination").and_then(|v| v.as_bool()) {
                policy.auto_continue.safe_branch_elimination = v;
            }
            if let Some(v) = ac.get("reversible_file_changes").and_then(|v| v.as_str()) {
                policy.auto_continue.reversible_file_changes = v.to_string();
            }
        }

        if let Some(rr) = doc.get("require_review") {
            if let Some(v) = rr.get("unverified_assumptions").and_then(|v| v.as_bool()) {
                policy.require_review.unverified_assumptions = v;
            }
            if let Some(v) = rr.get("irreversible_actions").and_then(|v| v.as_str()) {
                policy.require_review.irreversible_actions = v.to_string();
            }
            if let Some(v) = rr.get("contradictory_evidence").and_then(|v| v.as_bool()) {
                policy.require_review.contradictory_evidence = v;
            }
            if let Some(v) = rr.get("choosing_between_equal").and_then(|v| v.as_bool()) {
                policy.require_review.choosing_between_equal = v;
            }
        }

        if let Some(b) = doc.get("budget") {
            if let Some(v) = b.get("max_tool_calls_per_exploration").and_then(|v| v.as_integer()) {
                policy.budget.max_tool_calls_per_exploration = v as u32;
            }
            if let Some(v) = b.get("max_tokens_per_exploration").and_then(|v| v.as_integer()) {
                policy.budget.max_tokens_per_exploration = v as u64;
            }
            if let Some(v) = b.get("max_branches").and_then(|v| v.as_integer()) {
                policy.budget.max_branches = v as u32;
            }
            if let Some(v) = b.get("max_depth").and_then(|v| v.as_integer()) {
                policy.budget.max_depth = v as u32;
            }
        }

        Ok(policy)
    }

    pub fn to_budget(&self) -> ExplorationBudget {
        ExplorationBudget {
            max_tool_calls: self.budget.max_tool_calls_per_exploration,
            max_tokens: self.budget.max_tokens_per_exploration,
            max_branches: self.budget.max_branches,
            max_depth: self.budget.max_depth,
            used_tool_calls: 0,
            used_tokens: 0,
        }
    }

    /// Filters a convergence verdict through autonomy policy rules.
    /// Returns the filtered verdict, which may be escalated if review is required.
    pub fn filter_verdict(&self, mut verdict: ConvergenceVerdict, state: &ExplorationState) -> ConvergenceVerdict {
        // If policy requires review on unverified assumptions, and assumptions exist
        if self.require_review.unverified_assumptions
            && !state.assumptions.is_empty()
            && matches!(verdict.outcome, ConvergenceOutcome::Converged)
        {
            let count = state.assumptions.len();
            let summary = state.assumptions.iter().map(|a| format!("{}: {}", a.id, a.text)).collect::<Vec<_>>().join("; ");
            verdict.outcome = ConvergenceOutcome::Escalate(format!(
                "Autonomy policy requires human review: {count} unverified assumption(s) present ({summary})"
            ));
        }

        // If policy requires review on contradictions
        if self.require_review.contradictory_evidence
            && !verdict.contradictions.is_empty()
            && matches!(verdict.outcome, ConvergenceOutcome::Expand(_))
        {
            verdict.outcome = ConvergenceOutcome::Escalate(format!(
                "Autonomy policy requires review before re-expansion: {}",
                verdict.contradictions.join("; ")
            ));
        }

        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lantern::converge::evaluate_convergence;
    use crate::lantern::explore::ExplorationBudget;

    #[test]
    fn default_policy_allows_converged() {
        let policy = AutonomyPolicy::default();
        let budget = policy.to_budget();
        assert_eq!(budget.max_tool_calls, 20);
        assert_eq!(budget.max_depth, 2);

        let mut state = ExplorationState::new(budget);
        let q = state.open_question("Q1", None);
        let b = state.open_branch(&q, "B1", 0);
        let ev = state.add_evidence(&b, crate::lantern::events::EvidenceSource::File { path: "f".into() }, "ev", "data");
        let h = state.add_hypothesis(&q, "H1", vec![ev.clone()]);
        state.propose_decision(&q, &h, "Opt1", vec![ev], vec![], "done");

        let verdict = evaluate_convergence(&state);
        let filtered = policy.filter_verdict(verdict, &state);
        assert_eq!(filtered.outcome, ConvergenceOutcome::Converged);
    }

    #[test]
    fn policy_escalates_on_unverified_assumptions() {
        let mut policy = AutonomyPolicy::default();
        policy.require_review.unverified_assumptions = true;

        let mut state = ExplorationState::new(ExplorationBudget::default());
        let q = state.open_question("Q1", None);
        let b = state.open_branch(&q, "B1", 0);
        let ev = state.add_evidence(&b, crate::lantern::events::EvidenceSource::File { path: "f".into() }, "ev", "data");
        let h = state.add_hypothesis(&q, "H1", vec![ev.clone()]);
        let asm = state.declare_assumption("Assumption 1", "intuition", "low");
        state.propose_decision(&q, &h, "Opt1", vec![ev], vec![asm], "done");

        let verdict = evaluate_convergence(&state);
        assert_eq!(verdict.outcome, ConvergenceOutcome::Converged);

        let filtered = policy.filter_verdict(verdict, &state);
        assert!(matches!(filtered.outcome, ConvergenceOutcome::Escalate(_)));
    }

    #[test]
    fn policy_parses_toml() {
        let toml_content = r#"
[auto_continue]
read_only_investigation = true
safe_branch_elimination = false
reversible_file_changes = "ask"

[require_review]
unverified_assumptions = true
irreversible_actions = "always"
contradictory_evidence = true
choosing_between_equal = false

[budget]
max_tool_calls_per_exploration = 35
max_tokens_per_exploration = 80000
max_branches = 8
max_depth = 3
"#;
        let policy = AutonomyPolicy::parse_toml(toml_content).unwrap();
        assert!(!policy.auto_continue.safe_branch_elimination);
        assert_eq!(policy.auto_continue.reversible_file_changes, "ask");
        assert!(policy.require_review.unverified_assumptions);
        assert!(!policy.require_review.choosing_between_equal);
        assert_eq!(policy.budget.max_tool_calls_per_exploration, 35);
        assert_eq!(policy.budget.max_tokens_per_exploration, 80_000);
        assert_eq!(policy.budget.max_branches, 8);
        assert_eq!(policy.budget.max_depth, 3);
    }
}
