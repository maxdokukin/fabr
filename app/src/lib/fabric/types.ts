export const STATES = ['reminder', 'task', 'defined', 'in_progress', 'succeeded', 'failed'] as const;
export type State = (typeof STATES)[number];

export const STEPS = ['gather', 'define', 'plan', 'approve', 'execute', 'evaluate', 'learn', 'notify'] as const;
export type Step = (typeof STEPS)[number];

export const FIELDS = [
  'schema', 'id', 'source', 'title', 'due', 'pri', 'state', 'step', 'waiting', 'paused', 'suspended', 'suspend_until', 'created_at', 'updated_at', 'requests',
  'question', 'answers', 'grants', 'plan_meta', 'assertions', 'approval', 'verdict', 'cost_usd', 'links', 'by', 'needs',
] as const;

export const SCHEMA = 6;
export const OPTIONAL_FIELDS: readonly string[] = ['pri', 'paused', 'suspended', 'suspend_until', 'needs', 'by'];

export const SECTION_HEADINGS = ['Context', 'Spec', 'Success', 'Plan', 'Log'] as const;

export type QuestionKind = 'open' | 'choice' | 'grant';
export type Scope = 'task' | 'project' | 'always';

export type Redo = 'define' | 'plan';
export type Author = 'human' | 'agent' | null;
export interface By { context: Author; spec: Author; success: Author; plan: Author }
export const NO_AUTHOR: By = { context: null, spec: null, success: null, plan: null };
export interface Requires { tools: string[]; checks: string[] }

export interface Requests { delegate: boolean; cancel: boolean; redelegate: boolean; pause: boolean; resume: boolean; complete: boolean; redo: Redo | null }

export interface Answer {
  value: string | null;
  replacement: string | null;
  granted: { field: string; scope: Scope }[] | null;
  by: 'gui' | 'telegram';
  at: string;
}

export interface Question {
  id: string;
  kind: QuestionKind;
  text: string;
  options: string[] | null;
  prior: { text: string; source_task: string; last_confirmed: string } | null;
  fields: string[] | null;
  asked_at: string;
  answer: Answer | null;
}

export interface Grants { requested: string[]; granted: { field: string; scope: Scope }[] }

export interface BoundAction {
  tool: string;
  params: string;
  destination: string;
  effect: string;
  grant_class: string;
}

export interface PlanMeta {
  reversibility: 'reversible' | 'irreversible';
  tools: string[];
  requires: Requires;
  assumptions: { rule_id: string; text: string }[];
  checks?: { cmd: string; args: string[] }[];
  bound_actions: BoundAction[];
}

export interface Assertion { id: string; text: string }

export interface Approval {
  status: 'pending' | 'approved' | 'refused';
  by?: 'gui' | 'telegram' | 'auto' | null;
  at?: string | null;
  note?: string | null;
  digest?: string | null;
}

export interface TaskSource {
  node_path: string;
  todo_id: string;
}

export interface VerdictItem { id: string; pass: boolean; evidence: string }
export interface Verdict { result: 'pass' | 'fail'; at: string; items: VerdictItem[] }

export interface Links { redelegated_from: string | null; redelegated_to: string | null }

export interface FrontMatter {
  schema: number;
  id: string;
  source: TaskSource | null;
  title: string;
  due: string | null;
  pri: number;
  state: State;
  step: Step | null;
  waiting: boolean;
  needs: string | null;
  paused: boolean;
  suspended: boolean;
  suspend_until: string | null;
  created_at: string;
  updated_at: string;
  requests: Requests;
  question: Question | null;
  answers: Question[];
  grants: Grants;
  plan_meta: PlanMeta | null;
  assertions: Assertion[];
  by: By;
  approval: Approval | null;
  verdict: Verdict | null;
  cost_usd: number;
  links: Links;
}

export interface LogEntry { ts: string; step: string; message: string }

export interface Sections {
  context: string;
  spec: string;
  success: string;
  plan: string;
  log: string;
  logEntries: LogEntry[];
}

export interface TaskRecord extends FrontMatter { sections: Sections }
