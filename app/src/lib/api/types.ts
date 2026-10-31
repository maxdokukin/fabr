import type { Phase } from '../fabric/phase';

export type Pri = 0 | 1 | 2 | 3;

export interface Item {
  id: string;
  phase: Phase;
  due: string | null;
  title: string;
  pri: Pri;
}

export type MetaValue = string | number | boolean;
export type Meta = Record<string, MetaValue>;

export interface Node {
  source: string;
  path: string;
  name: string;
  managed: boolean;
  meta: Meta;
  items: Item[];
  warnings: string[];
  version: string | null;
  via: string | null;
  children: Node[];
}

export interface Source {
  id: string;
  name: string;
  host: string | null;
  root: string;
  prune: string[] | null;
  search_depth?: number | null;
  password?: string;
}

export interface Settings {
  poll_ms: number;
  remote_poll_ms: number;
  search_depth?: number;
  full_rescan_ms?: number;
  max_nodes?: number;
  log_path?: string | null;
}

export interface TestResult {
  ok: boolean;
  message: string;
  latency_ms: number;
  node_count: number;
  managed_root?: boolean;
  failure_kind?: FailureKind | null;
  detail?: string | null;
}

export type FailureKind = 'host_key_unknown' | 'host_key_changed' | 'auth_failed' | 'connect_failed' | 'login_required' | 'other';

export interface HostKey {
  type: string;
  fingerprint: string;
  line: string;
}

export interface TrustResult { added: number }
export interface PasswordState { stored: boolean }

export interface InstallKeyReq {
  source: string;
  host?: string | null;
  password?: string;
}

export interface InstallKeyResult {
  installed: boolean;
  generated_key: boolean;
  message: string;
}

export interface SourceError {
  source: string;
  message: string;
}

export interface Tree {
  root: Node;
  sources: Source[];
  today: string;
  truncated: boolean;
  truncated_reason?: string | null;
  truncated_by?: 'time' | 'visited' | 'nodes' | null;
  errors: SourceError[];
  pending?: string[];
}

export interface ListDirsReq {
  source: string;
  path: string;
}

export interface DirEntry {
  name: string;
  managed: boolean;
  pruned: boolean;
}

export interface InitChildrenReq extends NodeRef {
  names: string[];
}

export interface NodeRef {
  source: string;
  path: string;
}

export interface RemoveNodeReq {
  source: string;
  path: string;
  version: string;
}

export interface SaveMetaReq {
  source: string;
  path: string;
  version: string;
  meta: Meta;
}

export interface Stamp {
  stamp: string;
}

export interface FabricFile { path: string; mtime: number }
export interface FabricListReq { root?: string; home?: string | null }
export interface FabricListRes { records: FabricFile[]; registry: FabricFile[]; events: FabricFile | null; backend?: FabricFile | null }
export interface FabricReadRes { text: string; mtime: number }
export interface FabricWriteReq { path: string; text: string; expected_updated_at: string }
export interface FabricWriteRes { mtime: number }
export interface FabricCreateReq { folder: string; title: string; due?: string | null; pri?: number; context?: string; spec?: string; success?: string }
export interface ImportTodosReq { path: string }
export interface ImportTodosRes { created: number }
export interface FabricCreateRes { path: string; id: string }
export interface FabricRemoteFile { path: string; text: string }
export interface FabricRemoteListRes { records: FabricRemoteFile[]; registry: { name: string; text: string }[]; events: string; backend: string | null; stamp: string }
export interface FabricRemoteWriteReq { source: Source; path: string; text: string; expected_updated_at: string }
export interface FabricRemoteCreateReq { source: Source; folder: string; title: string; due?: string | null; pri?: number; context?: string; spec?: string; success?: string }
export interface FabricStampReq { roots: string[]; home?: string | null }

export interface ErrorDto {
  code: number;
  message: string;
  node: Node | null;
}

export class ApiError extends Error {
  code: number;
  node?: Node;
  constructor(code: number, message: string, node?: Node | null) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    if (node) this.node = node;
  }
  static from(e: unknown, fallbackCode = 500): ApiError {
    if (e instanceof ApiError) return e;
    if (e && typeof e === 'object' && 'message' in e) {
      const dto = e as Partial<ErrorDto>;
      return new ApiError(typeof dto.code === 'number' ? dto.code : fallbackCode, String(dto.message), dto.node ?? null);
    }
    if (typeof e === 'string') {
      try {
        const dto = JSON.parse(e) as Partial<ErrorDto>;
        if (dto && typeof dto.message === 'string') return new ApiError(typeof dto.code === 'number' ? dto.code : fallbackCode, dto.message, dto.node ?? null);
      } catch {}
      return new ApiError(fallbackCode, e);
    }
    return new ApiError(fallbackCode, 'command failed');
  }
}

export function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export interface SetupState { completed_steps: string[]; completed: boolean }
export type ExecuteMode = 'full' | 'read_only' | 'off';
export interface Capabilities { tools: string[]; checks: string[] }
export interface FabricSettings {
  default_folder: string; model: string; telegram_user_id: number | null; auto_mode: boolean;
  question_budget: number; heartbeat_interval_s: number; execute: ExecuteMode; capabilities: Capabilities; setup: SetupState;
  claude_bin: string; effort: string; max_budget_usd: number; execute_timeout_s: number;
}
export interface FabricSettingsRead { exists: boolean; settings: FabricSettings }
export interface FabricSecretsWriteReq { telegram_bot_token?: string }
export interface FabricSecretsStatus {
  telegram_bot_token: boolean;
  telegram_bot_token_fingerprint?: string | null;
}
export interface OpenSessionReq { folder: string; session_id: string }
export interface OpenClaudeReq { folder: string }
export interface CcSession { id: string; title: string; modified_at: string; messages: number; bytes: number }
export interface TranscriptEntry { role: 'user' | 'assistant' | 'tool'; text: string; tool?: string | null; at?: string | null }
export interface CcSendReq { folder: string; id: string; message: string }
export interface CcSendRes { reply: string; cost_usd: number }
export interface Transcript { id: string; title: string; entries: TranscriptEntry[]; truncated: boolean }
export interface LearnedRecord { id: string; text: string; applies_when: string; scope: string; count: number; source_tasks: string[]; last_confirmed?: string | null }
export type KnowledgeKind = 'rules' | 'knowledge';
export interface NodeKnowledge { rules: string; knowledge: string; learned: LearnedRecord[] }
export interface FabricAgentStatus { running: boolean; reason: string | null }
export type FabricCheckKind = 'folder' | 'claude' | 'telegram_token';
export interface FabricCheckReq { kind: FabricCheckKind; path?: string }
export type FabricCheckResult =
  | { ok: true; version?: string; path?: string; logged_in?: boolean; auth?: string; bot_name?: string; user_id?: number }
  | { ok: false; reason: string };
