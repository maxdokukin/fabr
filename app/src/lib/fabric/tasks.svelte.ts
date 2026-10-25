import { isTauri } from '../api/env';
import { createApi } from '../api/index';
import type { Api } from '../api/commands';
import type { Source } from '../api/types';
import type { AnswerInput } from './answer';
import { submitApproval, type ApprovalDecision } from './approval';
import { BACKEND_LABEL, backendState, heartbeatOf, type BackendState } from './backend';
import { loadReadPosition, parseEvents, reconcileReadPosition, safeReadPosition, saveReadPosition, unreadCount, type FabricEvent } from './events';
import { folderName, folderOf, nodeFolder, type Entry } from './folder';
import { createFabricSource } from './live';
import { RemoteSource, type RemoteApi } from './remote-source';
import { parseRecord } from './parse';
import { submitDone } from './done';
import { submitSuspend, submitUnsuspend } from './suspend';
import { submitRequest, type RequestKey } from './requests';
import { submitAssertions, submitFields, submitSection, type FieldsEdit, type Section } from './sections';
import type { CreateFields, FabricSource } from './source';
import { minuteOf } from '../model';
import { isActive, phaseOf, type Phase } from './phase';
import { buildAgents, leadOf, sortAgents, type AgentView } from './stage';
import { entryMap, findByTaskPath, loadRegistry, samePath } from './view';
import { submitAnswer } from './write';
import type { RegistryEntry } from './agents';

export type Held = Entry & { text: string; source: string | null };
export type FolderCounts = { todo: number; active: number; needs_you: number; needs_harness: number; suspended: number; done: number; failed: number; total: number };

const EMPTY: Held[] = [];

function itemsSig(records: Held[], now: string): string {
  return records.map((h) => `${h.path}|${h.record.title}|${h.record.due ?? ''}|${h.record.pri}|${phaseOf(h.record, now)}`).join('\n');
}

export function localRoots(sources: Source[] | undefined): string[] {
  return (sources ?? []).filter((s) => !s.host).map((s) => s.root);
}

export function remoteSources(sources: Source[] | undefined): Source[] {
  return (sources ?? []).filter((s) => !!s.host);
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

interface Slice { records: Held[]; registry: RegistryEntry[]; events: FabricEvent[]; errors: string[] }
interface Remote extends Slice { adapter: RemoteSource; sig: string; stamp: string; busy: boolean }

export class FabricStore {
  records = $state<Held[]>([]);
  registry = $state<RegistryEntry[]>([]);
  events = $state<FabricEvent[]>([]);
  heartbeat = $state<string | null>(null);
  heartbeats = $state<Record<string, string | null>>({});
  readPosition = $state(0);
  now = $state(new Date());
  loaded = $state(false);
  errors = $state<string[]>([]);
  actionError = $state<string | null>(null);
  openPath = $state<string | null>(null);
  openReadonly = $state(false);
  home = $state<string | null>(null);
  workerRoot = $state<string | null>(null);
  openedRoots = $state<string[]>([]);
  // Records discovered on disk that failed to parse, keyed by absolute path -> parse error.
  // A notice may point at one of these: the file IS on disk, it just could not be read, which
  // is a different (and recoverable) situation than a genuinely missing record.
  unreadable = $state<Map<string, string>>(new Map());
  onItemsChange: (() => void) | null = null;

  #source: FabricSource | null = null;
  #timer: ReturnType<typeof setInterval> | null = null;
  #last = '';
  #dead = false;
  #eventsKey = 'events';
  #itemsSig: string | null = null;
  #local: Slice = { records: [], registry: [], events: [], errors: [] };
  #remotes = new Map<string, Remote>();
  #remoteApi: RemoteApi | Promise<RemoteApi> | null;

  constructor(source: FabricSource | null = null, remoteApi: RemoteApi | null = null) {
    this.#source = source;
    this.#remoteApi = remoteApi;
  }

  backendOf(source: string | null): BackendState {
    return source === null ? this.backend : backendState(this.heartbeats[source] ?? null, this.now);
  }

  backend = $derived(backendState(this.heartbeat, this.now));
  agentReason = $state<string | null>(null);
  backendLabel = $derived(this.agentReason && this.backend !== 'connected' ? `${BACKEND_LABEL[this.backend]}: ${this.agentReason}` : BACKEND_LABEL[this.backend]);
  unread = $derived(unreadCount(this.events, this.readPosition));
  phases = $derived(new Map(this.records.map((h) => [h.path, phaseOf(h.record, minuteOf(this.now))] as const)));
  byFolder = $derived.by(() => {
    const m = new Map<string, Held[]>();
    for (const h of this.records) {
      const k = folderOf(h.path);
      m.set(k, [...(m.get(k) ?? []), h]);
    }
    return m;
  });
  #entries = $derived.by(() => {
    const m = new Map<string, RegistryEntry>();
    for (const s of this.#sourceIds()) {
      for (const [k, v] of entryMap(this.records.filter((h) => h.source === s), this.registry.filter((e) => (e.source ?? null) === s))) m.set(k, v);
    }
    return m;
  });
  agents = $derived.by(() => {
    const m = new Map<string, AgentView>();
    for (const s of this.#sourceIds()) {
      for (const [k, v] of buildAgents(this.records.filter((h) => h.source === s), this.registry.filter((e) => (e.source ?? null) === s), this.backendOf(s), this.now, this.phases)) m.set(k, v);
    }
    return m;
  });
  agentList = $derived(sortAgents([...this.agents.values()]));
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

  #sourceIds(): (string | null)[] {
    return [null, ...this.#remotes.keys()];
  }

  setSources(sources: Source[] | undefined): void {
    const want = remoteSources(sources);
    let changed = false;
    for (const id of [...this.#remotes.keys()]) {
      if (!want.some((s) => s.id === id)) { this.#remotes.delete(id); delete this.heartbeats[id]; changed = true; }
    }
    for (const s of want) {
      const sig = JSON.stringify([s.host, s.root]);
      if (this.#remotes.get(s.id)?.sig === sig) continue;
      this.#remoteApi ??= createApi();
      this.#remotes.set(s.id, { adapter: new RemoteSource(this.#remoteApi, s), sig, stamp: '', busy: false, records: [], registry: [], events: [], errors: [] });
      changed = true;
    }
    if (changed) { this.#publish(); void this.#tickRemotes(); }
  }

  #publish(): void {
    const slices = [this.#local, ...this.#remotes.values()];
    this.records = slices.flatMap((s) => s.records);
    this.registry = slices.flatMap((s) => s.registry);
    this.events = slices.length === 1 ? this.#local.events : slices.flatMap((s) => s.events).sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
    this.errors = slices.flatMap((s) => s.errors);
  }

  async start(treeRoots: string[] = [], sources: Source[] = []): Promise<void> {
    if (this.#timer) return;
    this.#dead = false;
    this.openedRoots = treeRoots.map(trimSlash);
    this.#timer = setInterval(() => void this.tick(), 1000);
    this.setSources(sources);
    try {
      if (!this.#source) {
        const api = await createApi();
        const settings = await api.fabricSettingsRead();
        this.workerRoot = settings.exists ? trimSlash(settings.settings.default_folder) : null;
        this.#source = await createFabricSource(settings, treeRoots);
        if (isTauri()) this.home = (await (await import('@tauri-apps/api/path')).homeDir()).replace(/\/+$/, '');
        else this.home = await api.fabricHome().then((h) => h.replace(/\/+$/, ''), () => null);
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
    const remotes = this.#tickRemotes();
    if (this.#source) {
      try {
        const st = await this.#source.stamp();
        if (st !== this.#last) {
          this.#last = st;
          await this.load();
        }
      } catch (e) {
        this.#local.errors = [(e as Error).message];
        this.#publish();
      }
    }
    await remotes;
  }

  async #tickRemotes(): Promise<void> {
    await Promise.all([...this.#remotes.entries()].map(async ([id, r]) => {
      if (r.busy) return;
      r.busy = true;
      try {
        const st = await r.adapter.stamp();
        if (st !== r.stamp) {
          await this.#loadRemote(id);
          r.stamp = st;
        }
      } catch (e) {
        r.errors = [`${r.adapter.source.name}: ${(e as Error).message}`];
        this.#publish();
      } finally {
        r.busy = false;
      }
    }));
  }

  async #loadRemote(id: string): Promise<void> {
    const r = this.#remotes.get(id);
    if (!r) return;
    const snap = await r.adapter.snapshot();
    if (this.#remotes.get(id) !== r) return;
    const ok: Held[] = [];
    const bad: string[] = [];
    for (const { path, text } of snap.records) {
      try { ok.push({ path, record: parseRecord(text), text, source: id }); }
      catch (e) { bad.push(`${folderName(folderOf(path))}: ${(e as Error).message}`); }
    }
    r.records = ok;
    r.errors = bad;
    r.registry = loadRegistry(snap.registry.map(({ name, text }) => ({ path: name, text }))).entries.map((e) => ({ ...e, source: id }));
    r.events = parseEvents(snap.events.split('\n')).map((e) => ({ ...e, source: id }));
    this.heartbeats[id] = snap.backend ? heartbeatOf(snap.backend) : null;
    this.#publish();
    const sig = itemsSig(this.records, minuteOf(this.now));
    const changed = this.#itemsSig !== null && sig !== this.#itemsSig;
    this.#itemsSig = sig;
    if (changed) this.onItemsChange?.();
  }

  async load(): Promise<void> {
    const src = this.#source;
    if (!src) return;
    const ok: Held[] = [];
    const bad: string[] = [];
    const unreadable = new Map<string, string>();
    for (const { path, text } of await src.listRecords()) {
      try { ok.push({ path, record: parseRecord(text), text, source: null }); }
      catch (e) {
        const reason = (e as Error).message;
        bad.push(`${folderName(folderOf(path))}: ${reason}`);
        unreadable.set(path, reason);
      }
    }
    this.unreadable = unreadable;
    this.#local = { ...this.#local, records: ok, errors: bad };
    this.#publish();
    const sig = itemsSig(this.records, minuteOf(this.now));
    const changed = this.#itemsSig !== null && sig !== this.#itemsSig;
    this.#itemsSig = sig;
    const registry = loadRegistry(await src.listRegistry()).entries;
    const events = parseEvents(await src.readEvents());
    this.#local = { records: ok, registry, events, errors: bad };
    this.#publish();
    this.readPosition = reconcileReadPosition(this.#eventsKey, this.readPosition, this.events.length);
    this.heartbeat = await src.readBackendHeartbeat();
    this.loaded = true;
    if (changed) this.onItemsChange?.();
  }

  folderFor(source: Source | undefined, path: string): string | null {
    return nodeFolder(source, path, this.home);
  }

  heldFor(folder: string | null, nodePath: string, id: string): Held | undefined {
    const direct = this.tasksFor(folder).find((h) => h.record.id === id);
    if (direct) return direct;
    const same = this.records.filter((h) => h.record.id === id);
    const rel = nodePath.replace(/^\/+|\/+$/g, '');
    if (rel) {
      const near = same.find((h) => {
        const f = folderOf(h.path).replace(/\/+$/, '');
        return f === rel || f.endsWith(`/${rel}`);
      });
      if (near) return near;
    }
    return same.length === 1 ? same[0] : undefined;
  }

  tasksFor(folder: string | null): Held[] {
    return (folder && this.byFolder.get(folder)) || EMPTY;
  }

  countsFor(folder: string | null): FolderCounts {
    const list = this.tasksFor(folder);
    const c: FolderCounts = { todo: 0, active: 0, needs_you: 0, needs_harness: 0, suspended: 0, done: 0, failed: 0, total: list.length };
    for (const h of list) {
      const p = this.phaseFor(h);
      if (p === 'todo' || p === 'suspended' || p === 'done' || p === 'failed') c[p]++;
      else if (isActive(p)) c.active++;
      if (p === 'needs_you') c.needs_you++;
      if (p === 'needs_harness') c.needs_harness++;
    }
    return c;
  }

  phaseFor(h: Held): Phase {
    return this.phases.get(h.path) ?? phaseOf(h.record, minuteOf(this.now));
  }

  entryFor(recordPath: string): RegistryEntry | null {
    return this.#entries.get(recordPath) ?? null;
  }

  agentsFor(folder: string | null): AgentView[] {
    return folder ? this.agentList.filter((v) => v.folder === folder) : [];
  }

  leadAgentFor(folder: string | null): AgentView | null {
    return leadOf(this.agentsFor(folder));
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

  open(path: string | null, readonly = false): void {
    this.openPath = path;
    this.openReadonly = path !== null && readonly;
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

  #heldAt(path: string): Held | undefined {
    return this.records.find((h) => h.path === path);
  }

  #sourceFor(source: string | null): FabricSource {
    if (source === null) return this.#source!;
    const r = this.#remotes.get(source);
    if (!r) throw new Error(`source ${source} is no longer configured`);
    return r.adapter;
  }

  #sourceOfPath(path: string): FabricSource {
    return this.#sourceFor(this.#heldAt(path)?.source ?? null);
  }

  #sourceIdOfFolder(folder: string): string | null {
    const known = this.tasksFor(folder)[0];
    if (known) return known.source;
    let best: string | null = null;
    let len = -1;
    for (const [id, r] of this.#remotes) {
      const root = r.adapter.source.root.trim().replace(/\/+$/, '');
      if ((folder === root || folder.startsWith(`${root}/`)) && root.length > len) { best = id; len = root.length; }
    }
    return best;
  }

  async #reread(path: string): Promise<void> {
    const old = this.#heldAt(path);
    const text = await this.#sourceFor(old?.source ?? null).readRecord(path);
    const held: Held = { path, record: parseRecord(text), text, source: old?.source ?? null };
    this.records = this.records.map((h) => (h.path === path ? held : h));
  }

  async answer(h: Held, input: AnswerInput): Promise<void> {
    await submitAnswer(this.#sourceFor(h.source), h.path, h.text, input);
    await this.#reread(h.path);
  }

  async approve(h: Held, d: ApprovalDecision): Promise<void> {
    await submitApproval(this.#sourceFor(h.source), h.path, h.text, d);
    await this.#reread(h.path);
  }

  async request(h: Held, key: RequestKey): Promise<void> {
    await submitRequest(this.#sourceFor(h.source), h.path, h.text, key);
    await this.#reread(h.path);
  }

  async editSection(path: string, section: Section, text: string): Promise<void> {
    await submitSection(this.#sourceOfPath(path), path, await this.#fresh(path), section, text);
    await this.#reread(path);
  }

  async editAssertions(path: string, text: string): Promise<void> {
    await submitAssertions(this.#sourceOfPath(path), path, await this.#fresh(path), text);
    await this.#reread(path);
  }

  async editFields(path: string, f: FieldsEdit): Promise<void> {
    await submitFields(this.#sourceOfPath(path), path, await this.#fresh(path), f);
    await this.#shelved(path);
  }

  async markDone(path: string): Promise<void> {
    const h = this.#heldAt(path);
    if (!h) throw new Error(`no loaded record for ${path}`);
    await submitDone(this.#sourceFor(h.source), h.path, h.text);
    await this.#reread(h.path);
    this.#itemsSig = itemsSig(this.records, minuteOf(this.now));
    this.onItemsChange?.();
  }

  async complete(h: Held): Promise<void> {
    if (this.phaseFor(h) === 'todo') return this.markDone(h.path);
    await submitRequest(this.#sourceFor(h.source), h.path, await this.#fresh(h.path), 'complete');
    await this.#reread(h.path);
  }

  async #fresh(path: string): Promise<string> {
    const h = this.#heldAt(path);
    if (!h) throw new Error(`no loaded record for ${path}`);
    return this.#sourceFor(h.source).readRecord(path);
  }

  async pause(path: string): Promise<void> {
    await submitRequest(this.#sourceOfPath(path), path, await this.#fresh(path), 'pause');
    await this.#reread(path);
  }

  async resume(path: string): Promise<void> {
    await submitRequest(this.#sourceOfPath(path), path, await this.#fresh(path), 'resume');
    await this.#reread(path);
  }

  async suspend(path: string, until: string | null): Promise<void> {
    await submitSuspend(this.#sourceOfPath(path), path, await this.#fresh(path), until);
    await this.#shelved(path);
  }

  async unsuspend(path: string): Promise<void> {
    await submitUnsuspend(this.#sourceOfPath(path), path, await this.#fresh(path));
    await this.#shelved(path);
  }

  async #shelved(path: string): Promise<void> {
    await this.#reread(path);
    this.#itemsSig = itemsSig(this.records, minuteOf(this.now));
    this.onItemsChange?.();
  }

  async createTask(folder: string, fields: CreateFields): Promise<Held> {
    const sid = this.#sourceIdOfFolder(folder);
    const { path } = await this.#sourceFor(sid).createRecord(folder, fields);
    if (sid === null) await this.load();
    else await this.#loadRemote(sid);
    const held = this.#heldAt(path);
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
    const source = this.#heldAt(path)?.source ?? null;
    if (source !== null && this.#remotes.has(source)) return this.#loadRemote(source);
    await this.#reread(path);
  }
}
