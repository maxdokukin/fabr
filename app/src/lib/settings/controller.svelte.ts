import { shouldInitRoot } from '../addnodes';
import { errText, type Source } from '../api/types';
import type { Store } from '../store.svelte';
import {
  buildSource, firstError, hasErrors, isUntouched, rowError, sourceStatus, suggestId, validateDraft, type DraftErrors, type SourceStatus,
} from './drafts';
import { SetupPanel } from './setuppanel.svelte';
import { SshFlow } from './sshflow.svelte';
import { advancedToText, validateAdvanced, type AdvField, type AdvancedText } from './units';

export type Page =
  | { kind: 'root' | 'appearance' | 'sources' | 'advanced' | 'keys' | 'claude' | 'telegram' }
  | { kind: 'source'; uid: number };

export const PAGE_TITLES: Record<Exclude<Page['kind'], 'source'>, string> = {
  root: 'Settings', appearance: 'Appearance', sources: 'Sources', advanced: 'Advanced', keys: 'Keyboard shortcuts', claude: 'Claude Code', telegram: 'Telegram',
};

let nextUid = 1;

export class Draft {
  uid = nextUid++;
  src = $state<Source>({ id: '', name: '', host: null, root: '', prune: null });
  pruneText = $state('');
  depthText = $state('');
  initRoot = $state(true);
  isNew: boolean;
  saved = '';
  flow: SshFlow;

  private taken: () => string[];

  constructor(api: Store['api'], src: Source, isNew: boolean, taken: () => string[] = () => [], onTrusted?: () => void | Promise<void>) {
    this.taken = taken;
    this.src = { ...src };
    this.pruneText = (src.prune ?? []).join(' ');
    this.depthText = src.search_depth ? String(src.search_depth) : '';
    this.isNew = isNew;
    this.flow = new SshFlow(api, () => ({ ...this.src, id: this.id }), onTrusted);
    if (!isNew) this.saved = this.snapshot();
  }

  get id(): string {
    return this.isNew ? suggestId(this.src, this.taken()) : this.src.id;
  }

  built(): Source {
    return buildSource({ ...($state.snapshot(this.src) as Source), id: this.id }, this.pruneText, this.depthText);
  }
  snapshot(): string {
    return JSON.stringify(this.built());
  }
  get dirty(): boolean {
    return this.isNew || this.snapshot() !== this.saved || !!this.flow.password;
  }
  get title(): string {
    return this.src.name.trim() || (this.isNew ? 'New source' : this.src.id);
  }
  get untouched(): boolean {
    return this.isNew && isUntouched($state.snapshot(this.src) as Source, this.pruneText, this.depthText);
  }
}

export class SettingsController {
  stack = $state<Page[]>([{ kind: 'root' }]);
  drafts = $state<Draft[]>([]);
  adv = $state<AdvancedText>({ poll: '', remote: '', rescan: '', depth: '', maxNodes: '' });
  advErrors = $state<Partial<Record<AdvField, string>>>({});
  errors = $state<DraftErrors>({});
  error = $state<string | null>(null);
  busy = $state(false);
  setup: SetupPanel;
  private store: Store;

  constructor(store: Store, setup: SetupPanel = new SetupPanel()) {
    this.store = store;
    this.setup = setup;
    this.reset();
  }

  reset(page: 'sources' | null = null): void {
    this.stack = page ? [{ kind: 'root' }, { kind: page }] : [{ kind: 'root' }];
    this.drafts = this.store.sources.map((s) => this.makeDraft(s, false));
    this.adv = advancedToText(this.store.settings);
    this.advErrors = {};
    this.errors = {};
    this.error = null;
    this.busy = false;
    this.setup.clearResults();
  }

  private makeDraft(src: Source, isNew: boolean): Draft {
    const d: Draft = new Draft(
      this.store.api, src, isNew,
      () => this.drafts.filter((y) => y !== d).map((y) => y.id),
      () => (d.isNew ? undefined : this.store.load()),
    );
    return d;
  }

  reconcile(): void {
    const t = this.store.tree;
    if (!t) return;
    for (const d of this.drafts) {
      const r = d.flow.result;
      if (!r || r.ok || d.flow.testing || d.flow.trusting || d.isNew) continue;
      if (t.sources.some((s) => s.id === d.src.id) && !t.errors.some((e) => e.source === d.src.id) && d.snapshot() === d.saved) d.flow.clearResult();
    }
  }

  get page(): Page {
    return this.stack[this.stack.length - 1];
  }
  get draft(): Draft | null {
    const p = this.page;
    return p.kind === 'source' ? (this.drafts.find((d) => d.uid === p.uid) ?? null) : null;
  }
  get title(): string {
    const p = this.page;
    if (p.kind === 'source') return this.draft?.title ?? 'Source';
    return PAGE_TITLES[p.kind];
  }
  get backLabel(): string {
    if (this.stack.length === 1) return '';
    const d = this.draft;
    if (d?.untouched) return 'Cancel';
    const prev = this.stack[this.stack.length - 2];
    return prev.kind === 'source' ? 'Back' : PAGE_TITLES[prev.kind];
  }

  push(p: Page): void {
    this.stack.push(p);
    this.errors = {};
    this.error = null;
    if (p.kind === 'source') void this.draft?.flow.loadStored();
  }

  addSource(): Draft {
    const d = this.makeDraft({ id: '', name: '', host: null, root: '', prune: null }, true);
    this.drafts.push(d);
    this.push({ kind: 'source', uid: d.uid });
    return d;
  }

  status(d: Draft): SourceStatus {
    const t = this.store.tree;
    return sourceStatus(d.src.id, d.flow.result, t?.errors ?? [], t?.sources.map((s) => s.id) ?? []);
  }

  async back(): Promise<boolean> {
    if (this.busy) return false;
    if (!(await this.leave())) return false;
    if (this.stack.length === 1) this.store.sheetOpen = false;
    else {
      this.stack.pop();
      this.errors = {};
      this.error = null;
    }
    return true;
  }

  async done(): Promise<boolean> {
    if (this.busy) return false;
    if (!(await this.leave())) return false;
    this.store.sheetOpen = false;
    return true;
  }

  private async leave(): Promise<boolean> {
    const p = this.page;
    if (p.kind === 'advanced') return this.commitAdvanced();
    if (p.kind === 'source') {
      const d = this.draft;
      if (d?.untouched) {
        this.discard(d);
        return true;
      }
      return this.persistSources();
    }
    return true;
  }

  private discard(d: Draft): void {
    this.drafts = this.drafts.filter((x) => x !== d);
    this.errors = {};
    this.error = null;
  }

  async persistSources(): Promise<boolean> {
    const d = this.draft;
    for (const x of this.drafts.filter((y) => y !== d && y.untouched)) this.discard(x);
    if (!this.drafts.some((x) => x.dirty)) return true;
    for (const x of this.drafts) if (x.isNew) x.src.id = x.id;
    for (const x of this.drafts) {
      const e = validateDraft(x.src, x.depthText, this.drafts.filter((y) => y !== x).map((y) => y.src.id));
      if (hasErrors(e)) {
        if (x !== d) this.stack = [{ kind: 'root' }, { kind: 'sources' }, { kind: 'source', uid: x.uid }];
        this.errors = e;
        this.error = rowError(x.title, firstError(e) ?? '');
        return false;
      }
    }
    this.error = null;
    this.errors = {};
    return this.#try(async () => {
      const out = this.drafts.map((x) => x.built());
      const toInit = this.drafts.filter((x) => shouldInitRoot(x.flow.result, x.initRoot)).map((x) => x.src.id);
      await this.store.saveSourceList(out);
      for (const x of this.drafts) {
        x.src = { ...x.built() };
        x.isNew = false;
        x.saved = x.snapshot();
        await x.flow.savePassword();
      }
      await this.store.initRoots(toInit);
      for (const x of this.drafts) {
        if (toInit.includes(x.src.id) && x.flow.result) x.flow.result = { ...x.flow.result, managed_root: true };
      }
    });
  }

  get canDelete(): boolean {
    const d = this.draft;
    return !!d && (d.isNew || this.drafts.length > 1);
  }

  async deleteDraft(): Promise<boolean> {
    const d = this.draft;
    if (!d || !this.canDelete) return false;
    const rest = this.drafts.filter((x) => x !== d);
    const saved = d.isNew || await this.#try(async () => {
      await this.store.saveSourceList(rest.filter((x) => !x.isNew).map((x) => x.built()));
      if (d.flow.stored) await d.flow.clearPassword();
    });
    if (!saved) return false;
    this.drafts = rest;
    this.stack.pop();
    this.errors = {};
    this.error = null;
    return true;
  }

  async commitAdvanced(): Promise<boolean> {
    const { values, errors } = validateAdvanced(this.adv);
    this.advErrors = errors;
    const cur = this.store.settings;
    const next = { ...cur, ...values };
    const changed = (Object.keys(values) as (keyof typeof values)[]).some((k) => values[k] !== cur[k]);
    const saved = !changed || await this.#try(async () => {
      await this.store.saveSettingsOnly(next);
      const t = advancedToText(this.store.settings);
      for (const f of Object.keys(t) as AdvField[]) if (!errors[f]) this.adv[f] = t[f];
    });
    return saved && Object.keys(errors).length === 0;
  }

  async #try(run: () => Promise<void>): Promise<boolean> {
    this.busy = true;
    try {
      await run();
      return true;
    } catch (e) {
      this.error = errText(e);
      return false;
    } finally {
      this.busy = false;
    }
  }
}
