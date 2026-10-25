import { isTauri } from '../api/env';
import { createApi } from '../api/index';
import type { Source } from '../api/types';
import type { AnswerInput } from './answer';
import { submitApproval, type ApprovalDecision } from './approval';
import { BACKEND_LABEL, backendState } from './backend';
import { loadReadPosition, parseEvents, reconcileReadPosition, safeReadPosition, saveReadPosition, unreadCount, type FabricEvent } from './events';
import { COLUMNS, columnOf, folderName, folderOf, nodeFolder, type ColumnId, type Entry } from './folder';
import { createFabricSource } from './live';
import { parseRecord } from './parse';
import { submitRequest, type RequestKey } from './requests';
import type { CreateFields, FabricSource } from './source';
import { findByTaskPath, loadRegistry, samePath } from './view';
import { submitAnswer } from './write';
import type { RegistryEntry } from './agents';

export type Held = Entry & { text: string };
export type FolderCounts = { byColumn: Record<ColumnId, number>; waiting: number; total: number };

const EMPTY: Held[] = [];

export function localRoots(sources: Source[] | undefined): string[] {
  return (sources ?? []).filter((s) => !s.host).map((s) => s.root);
}

const trimSlash = (p: string) => p.replace(/\/+$/, '');

// Path-segment-aware containment: canonical `path` lives at or beneath canonical `root`.
// Both are compared trailing-slash-trimmed so `/a/proj` is under `/a/proj` and `/a/proj/x`
// but not under `/a/project`.
export function under(root: string, path: string): boolean {
  const r = trimSlash(root);
  const p = trimSlash(path);
  return p === r || p.startsWith(`${r}/`);
}

export type Scope = { workerRoot: string | null; openedRoots: string[]; outOfScope: Held[] };

export class FabricStore {
  records = $state<Held[]>([]);
  registry = $state<RegistryEntry[]>([]);
  events = $state<FabricEvent[]>([]);
  heartbeat = $state<string | null>(null);
  readPosition = $state(0);
  now = $state(new Date());
  loaded = $state(false);
  errors = $state<string[]>([]);
  actionError = $state<string | null>(null);
  openPath = $state<string | null>(null);
  home = $state<string | null>(null);
  workerRoot = $state<string | null>(null);
  openedRoots = $state<string[]>([]);
  // Records discovered on disk that failed to parse, keyed by absolute path -> parse error.
  // A notice may point at one of these: the file IS on disk, it just could not be read, which
  // is a different (and recoverable) situation than a genuinely missing record.
  unreadable = $state<Map<string, string>>(new Map());

  #source: FabricSource | null = null;
  #timer: ReturnType<typeof setInterval> | null = null;
  #last = '';
  #dead = false;
  #eventsKey = 'events';

  constructor(source: FabricSource | null = null) {
    this.#source = source;
  }

  backend = $derived(backendState(this.heartbeat, this.now));
  backendLabel = $derived(BACKEND_LABEL[this.backend]);
  unread = $derived(unreadCount(this.events, this.readPosition));
  byFolder = $derived.by(() => {
    const m = new Map<string, Held[]>();
    for (const h of this.records) {
      const k = folderOf(h.path);
      m.set(k, [...(m.get(k) ?? []), h]);
    }
    return m;
  });
  #running = $derived.by(() => {
    const m = new Map<string, RegistryEntry[]>();
    for (const e of this.registry) {
      const held = findByTaskPath(this.records, e.task_path);
      const k = held ? folderOf(held.path) : folderOf(e.task_path);
      m.set(k, [...(m.get(k) ?? []), e]);
    }
    return m;
  });
  openRecord = $derived(this.records.find((h) => h.path === this.openPath) ?? null);

  // Read-only scope summary the toolbar/banner can read. `outOfScope` is the set of loaded
  // records whose folder is not under any opened root. With no opened roots there is nothing
  // to compare against, so it is empty (an injected-source/test run captures no scope).
  scope = $derived.by<Scope>(() => {
    const workerRoot = this.workerRoot;
    const openedRoots = this.openedRoots;
    const outOfScope = openedRoots.length
      ? this.records.filter((h) => !openedRoots.some((r) => under(r, folderOf(h.path))))
      : [];
    return { workerRoot, openedRoots, outOfScope };
  });

  // The worker's presence (heartbeat/registry) lives under a space other than the opened
  // project when a registry entry drives a record whose folder is not under any opened root.
  // Observable from the registry paths that `listRegistry` returns; no record writes.
  presenceElsewhere = $derived.by<boolean>(() => {
    const roots = this.openedRoots;
    if (!roots.length) return false;
    return this.registry.some((e) => {
      const held = findByTaskPath(this.records, e.task_path);
      const folder = held ? folderOf(held.path) : folderOf(e.task_path);
      return !roots.some((r) => under(r, folder));
    });
  });

  async start(treeRoots: string[] = []): Promise<void> {
    if (this.#timer) return;
    this.#dead = false;
    this.openedRoots = treeRoots.map(trimSlash);
    this.#timer = setInterval(() => void this.tick(), 1000);
    try {
      if (!this.#source) {
        const api = await createApi();
        const settings = await api.fabricSettingsRead();
        this.workerRoot = settings.exists ? trimSlash(settings.settings.default_folder) : null;
        this.#source = await createFabricSource(settings, treeRoots);
        if (isTauri()) this.home = (await (await import('@tauri-apps/api/path')).homeDir()).replace(/\/+$/, '');
      }
    } catch (e) {
      this.errors = [(e as Error).message];
      return;
    }
    if (this.#dead) return;
    this.#eventsKey = this.#source.eventsKey();
    this.readPosition = loadReadPosition(this.#eventsKey);
    await this.tick();
  }

  stop(): void {
    this.#dead = true;
    if (this.#timer) clearInterval(this.#timer);
    this.#timer = null;
  }

  async tick(): Promise<void> {
    this.now = new Date();
    if (!this.#source) return;
    try {
      const st = await this.#source.stamp();
      if (st !== this.#last) {
        this.#last = st;
        await this.load();
      }
    } catch (e) {
      this.errors = [(e as Error).message];
    }
  }

  async load(): Promise<void> {
    const src = this.#source;
    if (!src) return;
    const ok: Held[] = [];
    const bad: string[] = [];
    const unreadable = new Map<string, string>();
    for (const { path, text } of await src.listRecords()) {
      try { ok.push({ path, record: parseRecord(text), text }); }
      catch (e) {
        const reason = (e as Error).message;
        bad.push(`${folderName(folderOf(path))}: ${reason}`);
        unreadable.set(path, reason);
      }
    }
    this.records = ok;
    this.errors = bad;
    this.unreadable = unreadable;
    this.registry = loadRegistry(await src.listRegistry()).entries;
    this.events = parseEvents(await src.readEvents());
    this.readPosition = reconcileReadPosition(this.#eventsKey, this.readPosition, this.events.length);
    this.heartbeat = await src.readBackendHeartbeat();
    this.loaded = true;
  }

  folderFor(source: Source | undefined, path: string): string | null {
    return nodeFolder(source, path, this.home);
  }

  tasksFor(folder: string | null): Held[] {
    return (folder && this.byFolder.get(folder)) || EMPTY;
  }

  countsFor(folder: string | null): FolderCounts {
    const list = this.tasksFor(folder);
    const byColumn = Object.fromEntries(COLUMNS.map((c) => [c.id, 0])) as Record<ColumnId, number>;
    for (const h of list) byColumn[columnOf(h.record.state)]++;
    return { byColumn, waiting: list.filter((h) => h.record.waiting).length, total: list.length };
  }

  runningFor(folder: string | null): RegistryEntry | null {
    return (folder && this.#running.get(folder)?.[0]) || null;
  }

  entryFor(recordPath: string): RegistryEntry | null {
    return this.registry.find((e) => findByTaskPath(this.records, e.task_path)?.path === recordPath) ?? null;
  }

  titleFor = (taskPath: string): string | undefined => findByTaskPath(this.records, taskPath)?.record.title;

  locate(taskPath: string): Held | undefined {
    return findByTaskPath(this.records, taskPath);
  }

  // The parse error for a record that IS on disk but failed to load, or null when the path is
  // not among the unreadable records. Lets a notice distinguish "exists but unreadable" from
  // "genuinely missing" so the user gets an accurate, recoverable message.
  unreadableReason(taskPath: string): string | null {
    for (const [path, reason] of this.unreadable) {
      if (samePath(path, taskPath)) return reason;
    }
    return null;
  }

  open(path: string | null): void {
    this.openPath = path;
    this.actionError = null;
  }

  // Advance the local read position, but never past a still-actionable unread event so a
  // required action is never hidden by mark-read (Bug B, 2.4). An event is still actionable when
  // its resolved target record is waiting or has a pending approval. Touches only local
  // read-position state — no record writes (preserves 3.4).
  markAllRead(): void {
    this.readPosition = safeReadPosition(this.events, this.readPosition, (e) => {
      const record = this.locate(e.taskPath)?.record;
      return !!record && (record.waiting || record.approval?.status === 'pending');
    });
    saveReadPosition(this.#eventsKey, this.readPosition);
  }

  async #reread(path: string): Promise<void> {
    const text = await this.#source!.readRecord(path);
    const held = { path, record: parseRecord(text), text };
    this.records = this.records.map((h) => (h.path === path ? held : h));
  }

  async answer(h: Held, input: AnswerInput): Promise<void> {
    await submitAnswer(this.#source!, h.path, h.text, input);
    await this.#reread(h.path);
  }

  async approve(h: Held, d: ApprovalDecision): Promise<void> {
    await submitApproval(this.#source!, h.path, h.text, d);
    await this.#reread(h.path);
  }

  async request(h: Held, key: RequestKey): Promise<void> {
    await submitRequest(this.#source!, h.path, h.text, key);
    await this.#reread(h.path);
  }

  async createTask(folder: string, fields: CreateFields): Promise<Held> {
    const { path } = await this.#source!.createRecord(folder, fields);
    await this.load();
    const held = this.records.find((h) => h.path === path);
    if (!held) throw new Error(`created ${path} but could not read it back`);
    return held;
  }

  async createAndDelegate(folder: string, fields: CreateFields): Promise<Held> {
    const made = await this.createTask(folder, fields);
    await this.request(made, 'delegate');
    return this.records.find((h) => h.path === made.path)!;
  }

  async cancelTask(taskPath: string): Promise<void> {
    const h = this.locate(taskPath);
    if (!h) { this.actionError = `no loaded record for ${taskPath}`; return; }
    try { await this.request(h, 'cancel'); this.actionError = null; }
    catch (e) { this.actionError = (e as Error).message; }
  }

  async reload(path: string): Promise<void> {
    await this.#reread(path);
  }
}
