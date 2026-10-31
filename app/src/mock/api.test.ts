import { describe, expect, it } from 'vitest';
import { ApiError, type Node } from '../lib/api/types';
import { COMMANDS } from '../lib/api/commands';
import { createMockApi } from './api';
import { markStep } from '../lib/setup/wizard';
import { parseRecord } from '../lib/fabric/parse';
import { serializeRecord } from '../lib/fabric/serialize';

const count = (n: Node): number => 1 + n.children.reduce((a, c) => a + count(c), 0);
const find = (n: Node, path: string): Node | undefined =>
  n.path === path ? n : n.children.map((c) => find(c, path)).find(Boolean);

describe('mock api', () => {
  it('serves the 19-node sample (node_modules pruned) with the show_depth rule', async () => {
    const api = createMockApi({ today: '2026-10-19', latency: 0 });
    const t = await api.getTree(false);
    expect(t.root.children.length).toBe(1);
    expect([t.root.name, t.root.source, t.root.managed]).toEqual(['/', '', false]);
    expect(count(t.root.children[0])).toBe(18);
    expect(find(t.root, 'personal/finance/receipts')?.managed).toBe(false);
    expect(find(t.root, 'personal/finance/receipts/2026')?.managed).toBe(true);
    expect(find(t.root, 'work/frontend/node_modules')).toBeUndefined();
    expect(find(t.root, 'archive')?.meta.hidden).toBe(true);
  });

  it('multi-source: virtual root, via labels', async () => {
    const t = await createMockApi({ today: '2026-10-19', latency: 0, multi: true }).getTree(false);
    expect(t.root.name).toBe('/');
    expect(t.root.children.map((c) => c.source)).toEqual(['local', 'lab', 'vm']);
    expect(find(t.root.children[1], 'srv/wiki')?.via).toBe('srv/wiki');
  });

  it('saves bump the version and stamp; stale versions get 409 with the node', async () => {
    const api = createMockApi({ today: '2026-10-19', latency: 0 });
    const t = await api.getTree(false);
    const work = find(t.root, 'work')!;
    const s1 = (await api.getStamp()).stamp;
    const saved = await api.saveMeta({ source: 'local', path: 'work', version: work.version!, meta: { ...work.meta, kind: 'area' } });
    expect(saved.version).not.toBe(work.version);
    expect((await api.getStamp()).stamp).not.toBe(s1);
    const err = await api.saveMeta({ source: 'local', path: 'work', version: work.version!, meta: {} }).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe(409);
    expect(err.node.version).toBe(saved.version);
  });

  it('items look like records: record-style ids, pri 0..3, no has_todo, no saveTodo', async () => {
    const api = createMockApi({ today: '2026-10-19', latency: 0 });
    const work = find((await api.getTree(false)).root, 'work')!;
    expect(work.items[0]).toEqual({ id: '2026-10-19-submit-timesheet', phase: 'todo', due: '2026-10-20T17:00', title: 'Submit timesheet', pri: 2 });
    expect(work.items.every((i) => [0, 1, 2, 3].includes(i.pri))).toBe(true);
    expect('has_todo' in work).toBe(false);
    expect('saveTodo' in api).toBe(false);
  });

  it('fabric_write lets a reminder become succeeded (and logs it) but refuses other state changes', async () => {
    const api = createMockApi({ latency: 0 });
    const list = (await api.fabricList({})).records;
    const read = async (name: string) => {
      const path = list.find((r) => r.path.includes(name))!.path;
      return { path, text: (await api.fabricRead(path)).text };
    };
    const rem = await read('01-reminder');
    const rec = parseRecord(rem.text);
    const done = serializeRecord({ ...rec, state: 'succeeded', updated_at: '2030-01-01T00:00:00Z' });
    await api.fabricWrite({ path: rem.path, text: done, expected_updated_at: rec.updated_at });
    const after = parseRecord((await api.fabricRead(rem.path)).text);
    expect(after).toMatchObject({ state: 'succeeded', schema: 7 });
    expect(after.sections.logEntries.at(-1)?.message).toMatch(/done by hand/);
    const run = await read('05-running');
    const forged = serializeRecord({ ...parseRecord(run.text), state: 'succeeded' });
    const err = await api.fabricWrite({ path: run.path, text: forged, expected_updated_at: parseRecord(run.text).updated_at }).catch((e) => e);
    expect([err.code, err.message]).toEqual([403, expect.stringMatching(/state may not change/)]);
  });

  it('implements exactly the COMMANDS operations', () => {
    expect(Object.keys(createMockApi({ latency: 0 })).sort()).toEqual(Object.keys(COMMANDS).sort());
  });

  it('completing the folder and Claude Code steps flips setup.completed', async () => {
    const api = createMockApi({ latency: 0 });
    const first = await api.fabricSettingsRead();
    expect([first.exists, first.settings.setup.completed]).toEqual([false, false]);
    let cur = first.settings;
    for (const step of ['folder', 'claude'] as const) {
      cur = markStep(cur, step);
      const r = await api.fabricSettingsWrite(cur);
      expect(r.settings.setup.completed).toBe(step === 'claude');
    }
    expect(await api.fabricSettingsRead()).toMatchObject({ exists: true, settings: { setup: { completed: true } } });
  });

  it('a fresh config opens on setup and the two required steps open tasks', async () => {
    const { shouldShowSetup } = await import('../lib/setup/gate');
    const { ApiSetupSource } = await import('../lib/setup/setup-source');
    const api = createMockApi({ today: '2026-10-19', latency: 0 });
    const read = await api.fabricSettingsRead();
    expect(read.exists).toBe(false);
    expect(shouldShowSetup(read)).toBe(true);
    const src = new ApiSetupSource(api);
    let s = read.settings;
    for (const [step, kind] of [['folder', 'folder'], ['claude', 'claude']] as const) {
      expect((await src.check(kind, { path: '~/' })).ok).toBe(true);
      s = markStep(s, step);
    }
    expect(shouldShowSetup(await src.writeSettings(s))).toBe(false);
    expect(shouldShowSetup(await api.fabricSettingsRead())).toBe(false);
  });
});
