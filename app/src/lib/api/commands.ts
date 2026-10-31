import type {
  Tree, Stamp, Node, NodeRef, RemoveNodeReq, SaveMetaReq, Source, TestResult, Settings, ListDirsReq, DirEntry,
  InitChildrenReq, FabricListReq, FabricListRes, FabricReadRes, FabricWriteReq, FabricWriteRes, FabricStampReq, FabricRemoteListRes, FabricRemoteWriteReq, FabricRemoteCreateReq, FabricCreateReq, FabricCreateRes, ImportTodosReq, ImportTodosRes, HostKey, TrustResult, PasswordState, InstallKeyReq, InstallKeyResult,
  FabricSettings, FabricSettingsRead, FabricSecretsWriteReq, FabricSecretsStatus, FabricCheckReq, FabricCheckResult, FabricAgentStatus, OpenSessionReq,
  OpenClaudeReq, CcSession, Transcript, CcSendReq, CcSendRes, KnowledgeKind, NodeKnowledge,
} from './types';

export interface Command<A extends unknown[], R> {
  get?: true;
  req?: (...args: A) => unknown;
  readonly _res?: R;
}

const op = <A extends unknown[], R>(spec: { get?: true; req?: (...args: A) => unknown } = {}): Command<A, R> => spec;
const self = <T>(r: T) => r;

export const COMMANDS = {
  getTree: op<[showAll: boolean, rescan?: boolean], Tree>({ get: true, req: (show_all, rescan = false) => ({ show_all, rescan }) }),
  getStamp: op<[], Stamp>({ get: true }),
  saveMeta: op<[req: SaveMetaReq], Node>({ req: self }),
  initNode: op<[req: NodeRef], Node>({ req: self }),
  removeNode: op<[req: RemoveNodeReq], Node>({ req: self }),
  listSources: op<[], Source[]>({ get: true }),
  saveSources: op<[sources: Source[]], Source[]>({ req: (sources) => ({ sources }) }),
  testSource: op<[source: Source], TestResult>({ req: self }),
  getSettings: op<[], Settings>({ get: true }),
  saveSettings: op<[settings: Settings], Settings>({ req: self }),
  openPath: op<[req: NodeRef], void>({ req: self }),
  openUrl: op<[url: string], void>({ req: (url) => ({ url }) }),
  listDirs: op<[req: ListDirsReq], DirEntry[]>({ req: self }),
  initChildren: op<[req: InitChildrenReq], Node>({ req: self }),
  revealPath: op<[path: string], void>({ req: (path) => ({ path }) }),
  hostKey: op<[host: string], HostKey[]>({ req: (host) => ({ host }) }),
  trustHost: op<[host: string, lines: string[]], TrustResult>({ req: (host, lines) => ({ host, lines }) }),
  setSourcePassword: op<[source: string, password: string], PasswordState>({ req: (source, password) => ({ source, password }) }),
  hasSourcePassword: op<[source: string], PasswordState>({ req: (source) => ({ source }) }),
  fabricList: op<[req: FabricListReq], FabricListRes>({ req: self }),
  fabricRead: op<[path: string], FabricReadRes>({ req: (path) => ({ path }) }),
  fabricWrite: op<[req: FabricWriteReq], FabricWriteRes>({ req: self }),
  fabricCreate: op<[req: FabricCreateReq], FabricCreateRes>({ req: self }),
  importTodos: op<[req: ImportTodosReq], ImportTodosRes>({ req: self }),
  fabricStamp: op<[req: FabricStampReq], Stamp>({ req: self }),
  fabricRemoteList: op<[source: Source], FabricRemoteListRes>({ req: (source) => ({ source: source.id }) }),
  fabricRemoteStamp: op<[source: Source], { stamp: string }>({ req: (source) => ({ source: source.id }) }),
  fabricRemoteWrite: op<[req: FabricRemoteWriteReq], FabricWriteRes>({ req: (r) => ({ ...r, source: r.source.id }) }),
  fabricRemoteCreate: op<[req: FabricRemoteCreateReq], { path: string }>({ req: (r) => ({ ...r, source: r.source.id }) }),
  fabricSettingsRead: op<[], FabricSettingsRead>({}),
  fabricHome: op<[], string>({}),
  fabricSettingsWrite: op<[settings: FabricSettings], FabricSettingsRead>({ req: (settings) => ({ settings }) }),
  fabricSecretsWrite: op<[req: FabricSecretsWriteReq], FabricSecretsStatus>({ req: self }),
  fabricSecretsStatus: op<[], FabricSecretsStatus>({}),
  fabricCheck: op<[req: FabricCheckReq], FabricCheckResult>({ req: self }),
  fabricTelegramLink: op<[timeout_s?: number], FabricCheckResult>({ req: (timeout_s) => ({ timeout_s }) }),
  fabricOpenSession: op<[req: OpenSessionReq], void>({ req: self }),
  fabricOpenClaude: op<[req: OpenClaudeReq], void>({ req: self }),
  fabricCcSessions: op<[folder: string], CcSession[]>({ req: (folder) => ({ folder }) }),
  fabricCcTranscript: op<[folder: string, id: string, limit?: number], Transcript>({ req: (folder, id, limit = 300) => ({ folder, id, limit }) }),
  fabricCcSend: op<[req: CcSendReq], CcSendRes>({ req: self }),
  fabricKnowledgeRead: op<[folder: string], NodeKnowledge>({ req: (folder) => ({ folder }) }),
  fabricKnowledgeWrite: op<[folder: string, kind: KnowledgeKind, text: string], NodeKnowledge>({ req: (folder, kind, text) => ({ folder, kind, text }) }),
  fabricLearnedForget: op<[folder: string, id: string], NodeKnowledge>({ req: (folder, id) => ({ folder, id }) }),
  fabricAgentStatus: op<[], FabricAgentStatus>({}),
  fabricAgentStart: op<[], FabricAgentStatus>({}),
  installKey: op<[req: InstallKeyReq], InstallKeyResult>({ req: self }),
};

type Commands = typeof COMMANDS;

export type Api = {
  [K in keyof Commands]: Commands[K] extends Command<infer A, infer R> ? (...args: A) => Promise<R> : never;
};

export type Transport = (name: string, get: boolean, req: unknown) => Promise<unknown>;

export const snake = (key: string): string => key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

export function buildApi(transport: Transport, overrides: Partial<Api> = {}): Api {
  const out: Record<string, unknown> = {};
  for (const [key, spec] of Object.entries(COMMANDS) as [string, Command<unknown[], unknown>][]) {
    const name = snake(key);
    out[key] = (...args: unknown[]) => transport(name, !!spec.get, spec.req ? spec.req(...args) : undefined);
  }
  return { ...(out as Api), ...overrides };
}
