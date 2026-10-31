import { isoNow } from './answer';
import { parseRecord } from './parse';
import { serializeRecord } from './serialize';
import { SECTION_KEYS, FROZEN_MESSAGE, isEditable } from './sections';
import { NO_AUTHOR, NO_SESSIONS, SCHEMA, STEPS, type TaskRecord } from './types';

export interface RawRecord { path: string; text: string }

export interface CreateFields { title: string; due?: string | null; pri?: number; context?: string; spec?: string; success?: string }

export interface FabricSource {
  listRecords(): Promise<RawRecord[]>;
  readRecord(path: string): Promise<string>;
  writeRecord(path: string, text: string, expectedUpdatedAt: string): Promise<void>;
  createRecord(folder: string, fields: CreateFields): Promise<{ path: string; id: string }>;
  listRegistry(): Promise<RawRecord[]>;
  readEvents(): Promise<string[]>;
  eventsKey(): string;
  readBackendHeartbeat(): Promise<string | null>;
  stamp(): Promise<string>;
}

export function updatedAtOf(text: string): string {
  const lines = text.split('\n');
  if (lines[0]?.trimEnd() !== '---') return '';
  for (const l of lines.slice(1)) {
    if (l.trimEnd() === '---') break;
    if (l.startsWith('updated_at:')) return l.slice('updated_at:'.length).trim().replace(/^["']|["']$/g, '');
  }
  return '';
}

const sameJson = (x: unknown, y: unknown) => JSON.stringify(x) === JSON.stringify(y);

function guardEdits(a: TaskRecord, b: TaskRecord): TaskRecord {
  if (a.state !== b.state) return b;
  const sections = SECTION_KEYS.filter((s) => a.sections[s] !== b.sections[s]);
  const assertions = !sameJson(a.assertions, b.assertions);
  const fields = a.title !== b.title || a.due !== b.due || a.pri !== b.pri || !sameJson(a.grants.requested, b.grants.requested);
  if (!sections.length && !assertions && !fields) return b;
  if (!isEditable(a)) throw new Error(FROZEN_MESSAGE);
  const by = { ...b.by };
  for (const s of sections) by[s] = 'human';
  if (assertions) by.success = 'human';
  const replan = (sections.includes('spec') || sections.includes('success') || assertions) && (a.state === 'defined' || (a.state === 'task' && a.step !== null && STEPS.indexOf(a.step) > STEPS.indexOf('define')));
  return { ...b, by, requests: { ...b.requests, redo: b.requests.redo ?? (replan ? 'plan' : null) } };
}

export function guardState(before: string, after: string, now: Date = new Date()): string {
  const a = parseRecord(before);
  const b0 = parseRecord(after);
  if (a.paused !== b0.paused) throw new Error('400: paused is owned by the pipeline');
  const b = guardEdits(a, b0);
  if (a.state === b.state) return b === b0 ? after : serializeRecord(b);
  if (a.state !== 'reminder' || b.state !== 'succeeded') throw new Error(`state may not change from ${a.state} to ${b.state}`);
  const log = [b.sections.log, `- ${isoNow(now)} done: marked done by hand`].filter(Boolean).join('\n');
  return serializeRecord({ ...b, sections: { ...b.sections, log } });
}

const DUE = /^\d{4}-\d{2}-\d{2}(T([01]\d|2[0-3]):[0-5]\d)?$/;

export function taskSlug(title: string): string {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+/, '').slice(0, 40).replace(/-+$/, '');
  return slug || 'task';
}

export function newRecord(id: string, f: CreateFields, now: Date = new Date()): TaskRecord {
  const at = isoNow(now);
  return {
    schema: SCHEMA, id, source: null, title: f.title.trim(), due: f.due || null, pri: f.pri ?? 0, state: 'reminder', step: null, waiting: false, needs: null, paused: false, suspended: false, suspend_until: null,
    created_at: at, updated_at: at, requests: { delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null },
    question: null, answers: [], grants: { requested: [], granted: [] }, plan_meta: null, assertions: [], by: { ...NO_AUTHOR }, sessions: { ...NO_SESSIONS },
    approval: null, verdict: null, cost_usd: 0, links: { redelegated_from: null, redelegated_to: null },
    sections: { context: (f.context ?? '').trim(), spec: (f.spec ?? '').trim(), success: (f.success ?? '').trim(), plan: '', log: '', logEntries: [] },
  };
}

const files = import.meta.glob('../../../../fixtures/**/project/.fabric-space/tasks/*.md', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const registry = import.meta.glob('../../../../fixtures/**/home/.fabric-space/registry/*.yaml', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const events = import.meta.glob('../../../../fixtures/**/home/.fabric-space/events.log', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

export class FixtureSource implements FabricSource {
  private copy: Record<string, string> = { ...files };
  private writes = 0;

  async listRecords(): Promise<RawRecord[]> {
    return Object.keys(this.copy).sort().map((path) => ({ path, text: this.copy[path] }));
  }
  async readRecord(path: string): Promise<string> {
    if (!(path in this.copy)) throw new Error(`no record at ${path}`);
    return this.copy[path];
  }
  async writeRecord(path: string, text: string, expectedUpdatedAt: string): Promise<void> {
    const current = updatedAtOf(await this.readRecord(path));
    if (current !== expectedUpdatedAt) {
      throw new Error(`${path} changed: updated_at is "${current}", expected "${expectedUpdatedAt}"`);
    }
    this.copy[path] = guardState(this.copy[path], text);
    this.writes += 1;
  }
  async createRecord(folder: string, fields: CreateFields): Promise<{ path: string; id: string }> {
    if (!fields.title.trim()) throw new Error('a task needs a title');
    if (fields.due && !DUE.test(fields.due)) throw new Error(`bad date: ${fields.due}`);
    if (fields.pri !== undefined && !(Number.isInteger(fields.pri) && fields.pri >= 0 && fields.pri <= 3)) throw new Error(`bad priority: ${fields.pri}`);
    const dir = folder.includes('/') ? `${folder.replace(/\/+$/, '')}/.fabric-space/tasks` : `/fixtures/${folder}/project/.fabric-space/tasks`;
    const base = `${isoNow().slice(0, 10)}-${taskSlug(fields.title)}`;
    let id = base;
    for (let n = 2; `${dir}/${id}.md` in this.copy; n++) id = `${base}-${n}`;
    const path = `${dir}/${id}.md`;
    this.copy[path] = serializeRecord(newRecord(id, fields));
    this.writes += 1;
    return { path, id };
  }
  async listRegistry(): Promise<RawRecord[]> {
    return Object.keys(registry).sort().map((path) => ({ path, text: registry[path] }));
  }
  async readEvents(): Promise<string[]> {
    return Object.keys(events).sort().flatMap((p) => events[p].split('\n').filter((l) => l.trim() !== ''));
  }
  eventsKey(): string {
    return 'events';
  }
  async readBackendHeartbeat(): Promise<string | null> {
    return null;
  }
  async stamp(): Promise<string> {
    return String(this.writes);
  }
}

export function fixtureName(path: string): string {
  return /fixtures\/([^/]+)\//.exec(path)?.[1] ?? path;
}
