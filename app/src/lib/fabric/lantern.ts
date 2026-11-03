export type NodeKind =
  | 'goal'
  | 'question'
  | 'branch'
  | 'evidence'
  | 'hypothesis'
  | 'assumption'
  | 'decision'
  | 'gate'
  | 'action'
  | 'outcome'
  | 'steering';

export type NodeStatus = 'active' | 'resolved' | 'stale' | 'pruned' | 'eliminated';

export type EdgeKind = 'causal_parent' | 'evidence_for' | 'contradicts' | 'invalidates' | 'merges';

export interface LanternNode {
  id: string;
  kind: NodeKind;
  status: NodeStatus;
  depth: number;
  timestamp: string;
  label: string;
  data: Record<string, unknown>;
}

export interface LanternEdge {
  from: string;
  to: string;
  kind: EdgeKind;
}

export interface TimelineEntry {
  timestamp: string;
  event_id: string;
  width: number;
}

export interface LanternGraph {
  nodes: Record<string, LanternNode>;
  edges: LanternEdge[];
  timeline: TimelineEntry[];
}

export type SteeringAction =
  | { action: 'explore_further'; question_id: string }
  | { action: 'reject_assumption'; assumption_id: string }
  | { action: 'add_constraint'; text: string }
  | { action: 'prune_branch'; branch_id: string; reason: string }
  | { action: 'authorize_despite_gate'; gate_id: string; reason: string }
  | { action: 'redirect'; instructions: string }
  | { action: 'rewind_to'; checkpoint_id: string };

export interface SteeringReport {
  action_type: string;
  affected_nodes: string[];
  warnings: string[];
  details: string;
}

export async function fetchLanternGraph(taskId: string, runId?: string): Promise<LanternGraph> {
  const res = await fetch('/lantern/graph', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ task_id: taskId, run_id: runId }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to fetch Lantern graph (${res.status})`);
  }
  return res.json();
}

export async function applySteering(taskId: string, action: SteeringAction, runId?: string): Promise<SteeringReport> {
  const res = await fetch('/lantern/steer', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({ task_id: taskId, run_id: runId, action, by: 'user' }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to apply steering (${res.status})`);
  }
  return res.json();
}
