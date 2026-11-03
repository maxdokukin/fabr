use crate::lantern::events::LanternEvent;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Goal,
    Question,
    Branch,
    Evidence,
    Hypothesis,
    Assumption,
    Decision,
    Gate,
    Action,
    Outcome,
    Steering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Active,
    Resolved,
    Stale,
    Pruned,
    Eliminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    CausalParent,
    EvidenceFor,
    Contradicts,
    Invalidates,
    Merges,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanternNode {
    pub id: String,
    pub kind: NodeKind,
    pub status: NodeStatus,
    pub depth: u32,
    pub timestamp: String,
    pub label: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanternEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub timestamp: String,
    pub event_id: String,
    pub width: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LanternGraph {
    pub nodes: IndexMap<String, LanternNode>,
    pub edges: Vec<LanternEdge>,
    pub timeline: Vec<TimelineEntry>,
}

impl LanternGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_events(events: &[LanternEvent]) -> Self {
        let mut graph = LanternGraph::new();
        let mut active_branches = 0u32;

        for ev in events {
            let h = ev.header();
            for p in &h.parent_ids {
                graph.edges.push(LanternEdge { from: p.clone(), to: h.node_id.clone(), kind: EdgeKind::CausalParent });
            }

            match ev {
                LanternEvent::QuestionOpened { header, question_id, text, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: question_id.clone(),
                            kind: NodeKind::Question,
                            status: NodeStatus::Active,
                            depth: 0,
                            timestamp: header.timestamp.clone(),
                            label: format!("Question: {text}"),
                            data: serde_json::json!({ "question_id": question_id, "text": text }),
                        },
                    );
                }
                LanternEvent::BranchCreated { header, branch_id, description, .. } => {
                    active_branches = active_branches.saturating_add(1);
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: branch_id.clone(),
                            kind: NodeKind::Branch,
                            status: NodeStatus::Active,
                            depth: 1,
                            timestamp: header.timestamp.clone(),
                            label: format!("Branch: {description}"),
                            data: serde_json::json!({ "branch_id": branch_id, "description": description }),
                        },
                    );
                }
                LanternEvent::EvidenceCollected { header, evidence_id, summary, source, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: evidence_id.clone(),
                            kind: NodeKind::Evidence,
                            status: NodeStatus::Resolved,
                            depth: 2,
                            timestamp: header.timestamp.clone(),
                            label: format!("Evidence: {summary}"),
                            data: serde_json::json!({ "evidence_id": evidence_id, "summary": summary, "source": source }),
                        },
                    );
                }
                LanternEvent::HypothesisFormed { header, hypothesis_id, description, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: hypothesis_id.clone(),
                            kind: NodeKind::Hypothesis,
                            status: NodeStatus::Active,
                            depth: 1,
                            timestamp: header.timestamp.clone(),
                            label: format!("Candidate: {description}"),
                            data: serde_json::json!({ "hypothesis_id": hypothesis_id, "description": description }),
                        },
                    );
                }
                LanternEvent::HypothesisEliminated { header, hypothesis_id, reason, .. } => {
                    if let Some(node) = graph.nodes.values_mut().find(|n| n.id == *hypothesis_id) {
                        node.status = NodeStatus::Eliminated;
                    }
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: format!("{hypothesis_id}-elim"),
                            kind: NodeKind::Hypothesis,
                            status: NodeStatus::Eliminated,
                            depth: 1,
                            timestamp: header.timestamp.clone(),
                            label: format!("Eliminated {hypothesis_id}: {reason}"),
                            data: serde_json::json!({ "hypothesis_id": hypothesis_id, "reason": reason }),
                        },
                    );
                }
                LanternEvent::AssumptionDeclared { header, assumption_id, text, risk, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: assumption_id.clone(),
                            kind: NodeKind::Assumption,
                            status: NodeStatus::Active,
                            depth: 1,
                            timestamp: header.timestamp.clone(),
                            label: format!("Assumption: {text} [Risk: {risk}]"),
                            data: serde_json::json!({ "assumption_id": assumption_id, "text": text, "risk": risk }),
                        },
                    );
                }
                LanternEvent::DecisionProposed { header, decision_id, selected_option, stopping_rationale, .. } => {
                    active_branches = active_branches.saturating_sub(1).max(1);
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: decision_id.clone(),
                            kind: NodeKind::Decision,
                            status: NodeStatus::Resolved,
                            depth: 0,
                            timestamp: header.timestamp.clone(),
                            label: format!("Decision: {selected_option}"),
                            data: serde_json::json!({ "decision_id": decision_id, "selected_option": selected_option, "stopping_rationale": stopping_rationale }),
                        },
                    );
                }
                LanternEvent::DecisionStale { decision_id, cause, .. } => {
                    if let Some(node) = graph.nodes.values_mut().find(|n| n.id == *decision_id) {
                        node.status = NodeStatus::Stale;
                        node.label.push_str(&format!(" [STALE: {cause}]"));
                    }
                }
                LanternEvent::GateEvaluated { header, gate_id, verdict, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: gate_id.clone(),
                            kind: NodeKind::Gate,
                            status: NodeStatus::Resolved,
                            depth: 0,
                            timestamp: header.timestamp.clone(),
                            label: format!("Gate: {:?}", verdict.outcome),
                            data: serde_json::to_value(verdict).unwrap_or_default(),
                        },
                    );
                }
                LanternEvent::SteeringApplied { header, steering_id, action, target_node, detail, .. } => {
                    graph.nodes.insert(
                        header.node_id.clone(),
                        LanternNode {
                            id: steering_id.clone(),
                            kind: NodeKind::Steering,
                            status: NodeStatus::Resolved,
                            depth: 0,
                            timestamp: header.timestamp.clone(),
                            label: format!("Steer: {action} on {target_node} ({detail})"),
                            data: serde_json::json!({ "action": action, "target_node": target_node, "detail": detail }),
                        },
                    );
                }
                _ => {}
            }

            graph.timeline.push(TimelineEntry { timestamp: h.timestamp.clone(), event_id: h.event_id.clone(), width: active_branches });
        }

        graph
    }

    pub fn render_ascii(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Fabr Lantern Decision Graph ===\n\n");

        for node in self.nodes.values() {
            let indent = "  ".repeat(node.depth as usize);
            let status_mark = match node.status {
                NodeStatus::Active => "[? investigating]",
                NodeStatus::Resolved => "[✓ resolved]",
                NodeStatus::Stale => "[⚠ stale]",
                NodeStatus::Pruned => "[~ pruned]",
                NodeStatus::Eliminated => "[✗ eliminated]",
            };
            out.push_str(&format!("{indent}{status_mark} {}\n", node.label));
        }

        out.push_str("\n--- Lantern Shape Timeline ---\n");
        for t in &self.timeline {
            let bar = "█".repeat(t.width as usize);
            out.push_str(&format!("{} | {:<12} (width: {})\n", t.timestamp, bar, t.width));
        }

        out
    }

    pub fn causal_descendants(&self, target_id: &str) -> Vec<String> {
        let mut descendants = Vec::new();
        let mut queue = vec![target_id.to_string()];

        while let Some(current) = queue.pop() {
            for edge in &self.edges {
                if edge.from == current && !descendants.contains(&edge.to) {
                    descendants.push(edge.to.clone());
                    queue.push(edge.to.clone());
                }
            }
        }

        descendants
    }
}
