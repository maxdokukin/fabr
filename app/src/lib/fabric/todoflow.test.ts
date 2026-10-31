import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import type { Api } from '../api/index';
import type { Item, Node, Tree } from '../api/types';
import { boxModel } from '../model';
import NodeCard from '../tree/NodeCard.svelte';
import NodeSettingsSheet from '../settings/NodeSettingsSheet.svelte';
import { readFileSync } from 'node:fs';
import { Store } from '../store.svelte';
import { createMockApi } from '../../mock/api';
import { AddFlow } from './addflow.svelte';
import { applyDone, canMarkDone, submitDone } from './done';
import { folderOf } from './folder';
import { parseRecord } from './parse';
import { FixtureSource, guardState } from './source';
import { serializeRecord } from './serialize';
import { FabricStore } from './tasks.svelte';
import TaskSection from './TaskSection.svelte';
import RecordScreen from './RecordScreen.svelte';
import { applyRequest, canComplete, COMPLETE_PENDING } from './requests';

const FOLDER = '/home/me/p';
const VM = { id: 'vm', name: 'VM', host: 'vm', root: '~/sample', prune: null };

async function remoteStore() {
  const fabric = new FabricStore(new FixtureSource(), createMockApi({ today: '2026-10-25', latency: 0 }));
  await fabric.load();
  fabric.setSources([VM]);
  await vi.waitFor(() => expect(fabric.records.some((h) => h.source === 'vm')).toBe(true));
  return { fabric };
}
const today = '2026-10-25';

async function fresh() {
  const source = new FixtureSource();
  const fabric = new FabricStore(source);
  await fabric.load();
  return { source, fabric };
}

describe('AddFlow', () => {
  it('Enter with a title opens the sheet; an empty title, a missing folder or a disabled flow do not', async () => {
    const { fabric } = await fresh();
    const flow = new AddFlow(fabric, () => FOLDER);
    expect(flow.press('Enter')).toBe(true);
    expect(flow.sheet).toBe(false);
    flow.text = '  Pay rent ';
    flow.press('Enter');
    expect(flow.sheet).toBe(true);
    expect(new AddFlow(fabric, () => null).sheet).toBe(false);
    const nofolder = new AddFlow(fabric, () => null);
    nofolder.text = 'x';
    nofolder.ask();
    expect(nofolder.sheet).toBe(false);
    const off = new AddFlow(fabric, () => FOLDER, () => true);
    off.text = 'x';
    off.press('Enter');
    expect(off.sheet).toBe(false);
  });

  it('blur with text opens the sheet, Escape in the field clears it, cancel keeps the text', async () => {
    const { fabric } = await fresh();
    const flow = new AddFlow(fabric, () => FOLDER);
    flow.text = 'Call the bank';
    flow.ask();
    expect(flow.sheet).toBe(true);
    flow.cancel();
    expect(flow.sheet).toBe(false);
    expect(flow.text).toBe('Call the bank');
    expect(flow.press('Escape')).toBe(true);
    expect(flow.text).toBe('');
  });

  it('Save writes a reminder with pri and a timed due, then opens it', async () => {
    const { fabric } = await fresh();
    const flow = new AddFlow(fabric, () => FOLDER);
    flow.text = 'Pay rent';
    flow.ask();
    await flow.finish({ title: 'Pay rent', due: '2026-11-01T09:30', pri: 2, context: 'ctx' }, false);
    const [h] = fabric.tasksFor(FOLDER);
    expect(h.record).toMatchObject({ state: 'reminder', title: 'Pay rent', due: '2026-11-01T09:30', pri: 2, schema: 7 });
    expect(h.record.requests.delegate).toBe(false);
    expect(h.text).toMatch(/^pri: 2$/m);
    expect(flow.sheet).toBe(false);
    expect(flow.text).toBe('');
    expect(fabric.openPath).toBe(h.path);
  });

  it('Generate with AI also sets requests.delegate', async () => {
    const { fabric } = await fresh();
    const flow = new AddFlow(fabric, () => FOLDER);
    flow.text = 'Plan trip';
    flow.ask();
    await flow.finish({ title: 'Plan trip', pri: 1 }, true);
    expect(fabric.tasksFor(FOLDER)[0].record).toMatchObject({ state: 'reminder', pri: 1, requests: { delegate: true } });
  });

  it('keeps the sheet open and shows the error when the source refuses', async () => {
    const { fabric } = await fresh();
    const flow = new AddFlow(fabric, () => FOLDER);
    flow.text = 'x';
    flow.ask();
    await flow.finish({ title: 'x', due: 'tomorrow' }, false);
    expect(flow.sheet).toBe(true);
    expect(flow.error).toMatch(/bad date/);
    expect(fabric.tasksFor(FOLDER)).toHaveLength(0);
  });
});

describe('mark done', () => {
  it('only a reminder can be ticked', async () => {
    const { fabric } = await fresh();
    const running = fabric.records.find((h) => h.record.state === 'in_progress')!.record;
    expect(canMarkDone(running)).toBe(false);
    expect(() => applyDone(running)).toThrow(/only a reminder/);
    expect(canMarkDone(fabric.records.find((h) => h.record.state === 'reminder')!.record)).toBe(true);
  });

  it('writes state succeeded as the only state change; the mock core adds the Log line', async () => {
    const { source, fabric } = await fresh();
    await fabric.createTask(FOLDER, { title: 'Pay rent', pri: 3 });
    const h = fabric.tasksFor(FOLDER)[0];
    const writes = vi.spyOn(source, 'writeRecord');
    await fabric.markDone(h.path);
    expect(writes).toHaveBeenCalledTimes(1);
    const sent = parseRecord(writes.mock.calls[0][1]);
    expect(sent).toMatchObject({ state: 'succeeded', title: 'Pay rent', pri: 3, schema: 7 });
    expect(sent.sections.log).toBe('');
    const after = fabric.tasksFor(FOLDER)[0].record;
    expect(after.state).toBe('succeeded');
    expect(after.sections.logEntries.at(-1)?.message).toMatch(/done by hand/);
  });

  it('refuses any other state change through the source', async () => {
    const { source, fabric } = await fresh();
    const run = fabric.records.find((h) => h.record.state === 'in_progress')!;
    const forged = serializeRecord({ ...run.record, state: 'succeeded' });
    await expect(source.writeRecord(run.path, forged, run.record.updated_at)).rejects.toThrow(/state may not change/);
    expect(() => guardState(run.text, forged)).toThrow();
    await expect(submitDone(source, run.path, run.text)).rejects.toThrow(/only a reminder/);
  });

  it('shows the tick on every open row and moves a ticked row into the collapsed done section', async () => {
    const { fabric } = await fresh();
    await fabric.createTask(FOLDER, { title: 'Pay rent' });
    const src = fabric.records.find((h) => h.record.state === 'in_progress')!;
    fabric.records = [...fabric.records, { ...src, path: `${FOLDER}/.fabric-space/tasks/active.md`, record: { ...src.record, title: 'Order filament' } }];
    let body = render(TaskSection, { props: { fabric, folder: FOLDER } }).body;
    expect(body.match(/data-testid="mark-done"/g)).toHaveLength(2);
    expect(body.match(/data-testid="task-row" data-state="reminder"/g)).toHaveLength(1);
    expect(body.indexOf('Order filament')).toBeLessThan(body.indexOf('Pay rent'));
    expect(body).toContain('1 todo · 1 active');
    expect(body).not.toContain('data-testid="task-done"');
    await fabric.markDone(fabric.tasksFor(FOLDER).find((h) => h.record.title === 'Pay rent')!.path);
    body = render(TaskSection, { props: { fabric, folder: FOLDER } }).body;
    expect(body.match(/data-testid="mark-done"/g)).toHaveLength(1);
    expect(body).toContain('0 todo · 1 active');
    const done = body.slice(body.indexOf('data-testid="task-done"'));
    expect(done).toContain('Pay rent');
    expect(done).not.toContain('Order filament');
    expect(body).not.toMatch(/<details[^>]*open/);
  });
});

describe('items change hook', () => {
  it('fires for a created or ticked record but not for the first load or a Log-only change', async () => {
    const { source, fabric } = await fresh();
    const seen = vi.fn();
    fabric.onItemsChange = seen;
    await fabric.load();
    expect(seen).not.toHaveBeenCalled();
    await fabric.createTask(FOLDER, { title: 'Pay rent' });
    expect(seen).toHaveBeenCalledTimes(1);
    const h = fabric.tasksFor(FOLDER)[0];
    const noisy = serializeRecord({ ...h.record, updated_at: '2030-01-01T00:00:00Z', sections: { ...h.record.sections, log: '- 2030-01-01T00:00:00Z gather: hello' } });
    await source.writeRecord(h.path, noisy, h.record.updated_at);
    await fabric.load();
    expect(seen).toHaveBeenCalledTimes(1);
    await fabric.markDone(h.path);
    expect(seen).toHaveBeenCalledTimes(2);
  });

  it('Store.reloadSoon debounces into one tree load', async () => {
    vi.useFakeTimers();
    const getTree = vi.fn(async () => ({ root: node(''), sources: [], today, truncated: false, errors: [] }) as Tree);
    const store = new Store({ getTree, getStamp: async () => ({ stamp: 's' }) } as unknown as Api);
    store.reloadSoon();
    store.reloadSoon();
    store.reloadSoon();
    expect(getTree).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(300);
    expect(getTree).toHaveBeenCalledTimes(1);
    store.reloadSoon();
    store.stop();
    await vi.advanceTimersByTimeAsync(300);
    expect(getTree).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});

const item = (o: Partial<Item> = {}): Item => ({ id: 'i', phase: 'todo', due: null, title: 'Item', pri: 0, ...o });
const node = (path: string, o: Partial<Node> = {}): Node => ({
  source: 'local', path, name: path || 'root', managed: true, meta: {}, items: [], warnings: [], version: 'v', via: null, children: [], ...o,
});

describe('remote and local nodes', () => {
  async function storeFor(host: string | null) {
    const work = node('work', { items: [item({ id: 'a', title: 'Reply to vendor', phase: 'todo', pri: 1 }), item({ id: 'b', title: 'Migrate auth', phase: 'working' })] });
    const tree: Tree = {
      root: { ...node('', { managed: false, source: '' }), children: [node('', { children: [work] })] },
      sources: [{ id: 'local', name: 'Lab', host, root: '/srv/x', prune: null }],
      today, truncated: false, errors: [],
    };
    const store = new Store({ getTree: vi.fn(async () => tree), getStamp: vi.fn(async () => ({ stamp: 's' })) } as unknown as Api);
    await store.load();
    return store;
  }

  it('the node settings sheet lists no tasks for a remote or a local node', async () => {
    for (const host of ['lab.local', null]) {
      const store = await storeFor(host);
      store.openNodeSettings('local:work');
      const body = render(NodeSettingsSheet, { props: { store } }).body;
      expect(body).not.toContain('Reply to vendor');
      expect(body).not.toContain('remote-tasks');
    }
  });

  it('the node settings sheet shows the name, the Hide row and the meta editor for a managed node', async () => {
    const store = await storeFor(null);
    store.openNodeSettings('local:work');
    const body = render(NodeSettingsSheet, { props: { store } }).body;
    expect(body).toContain('Node settings');
    expect(body).toContain('work');
    expect(body).toContain('>Hide<');
    expect(body).toContain('Identity');
    expect(body).toContain('Remove node');
    expect(store.nodeSettings).not.toBeNull();
  });

  it('the virtual root settings list its sources instead of meta', async () => {
    const store = await storeFor(null);
    store.openNodeSettings(':');
    const body = render(NodeSettingsSheet, { props: { store } }).body;
    expect(body).toContain('Sources');
    expect(body).not.toContain('Identity');
  });

  it('a card gear button opens settings through a propagation-stopping handler', async () => {
    const n = node('work');
    const body = card(n, { onsettings() {} });
    expect(body).toContain('title="Node settings"');
    expect(card(n)).not.toContain('Node settings');
    const src = readFileSync(new URL('../tree/NodeCard.svelte', import.meta.url), 'utf8');
    expect(src).toContain('onclick={stop(onsettings)}');
    expect(src).not.toMatch(/function openItem[^}]*onopen\(\)/);
  });

  it('a remote card row renders enabled with a tick, and its add input is enabled', async () => {
    const { fabric } = await remoteStore();
    const n = node('', { items: [item({ id: '2026-10-25-check-printer-firmware', title: 'Check printer firmware', phase: 'todo' })] });
    const body = card(n, { fabric, folder: '~/sample' });
    expect(body).not.toContain('aria-disabled');
    expect(body).not.toContain('Remote task');
    expect(body).not.toContain('Remote node');
    expect(body).toContain('Mark Check printer firmware done');
    expect(body).not.toMatch(/<input[^>]*disabled[^>]*>/);
  });

  function card(n: Node, extra: Record<string, unknown> = {}) {
    const model = boxModel(n, { today, filter: 'all', hideDone: false, collapsed: new Set(), forceOpen: new Set(), moreOpen: new Set() } as never);
    return render(NodeCard, { props: { node: n, model, selected: false, match: false, dim: false, ontoggle() {}, onmore() {}, onopen() {}, onfocus() {}, today, ...extra } }).body;
  }

  it('a card reads No tasks, shows node warnings as they are, and has no status cycling', async () => {
    const { fabric } = await fresh();
    const warn = '.fabr/.todo found; 3 lines can be imported';
    const body = card(node('work', { warnings: [warn] }), { fabric, folder: FOLDER });
    expect(body).toContain('No tasks');
    expect(body).not.toContain('.todo file');
    expect(body).toContain(warn);
    expect(body).toContain('placeholder="Add task"');
    expect(body).not.toContain('click to change status');
  });

  it('a local card shows the tick for open rows that have a record', async () => {
    const { fabric } = await fresh();
    await fabric.createTask(FOLDER, { title: 'Pay rent' });
    const src = fabric.records.find((h) => h.record.state === 'in_progress')!;
    fabric.records = [...fabric.records, { ...src, path: `${FOLDER}/.fabric-space/tasks/active.md`, record: { ...src.record, id: 'active', title: 'Order filament' } }];
    const reminder = fabric.tasksFor(FOLDER).find((h) => h.record.state === 'reminder')!;
    const n = node('work', {
      items: [item({ id: reminder.record.id, title: 'Pay rent', phase: 'todo' }), item({ id: 'active', title: 'Order filament', phase: 'working' })],
    });
    const body = card(n, { fabric, folder: FOLDER });
    expect(body.match(/aria-label="Mark [^"]* done"/g)).toEqual(['aria-label="Mark Pay rent done"', 'aria-label="Mark Order filament done"']);
    expect(body).toContain('>active<');
  });
});

describe('pause and suspend through the store', () => {
  it('Pause writes requests.pause only; Resume needs a paused record', async () => {
    const { source, fabric } = await fresh();
    const run = fabric.records.find((h) => h.record.state === 'in_progress')!;
    await fabric.pause(run.path);
    const after = parseRecord(await source.readRecord(run.path));
    expect(after.requests).toEqual({ ...run.record.requests, pause: true });
    expect({ ...after, updated_at: '', requests: run.record.requests }).toEqual({ ...run.record, schema: 7, updated_at: '' });
    await expect(fabric.resume(run.path)).rejects.toThrow(/paused/);
  });
  it('Suspend writes suspended and suspend_until, Unsuspend clears them; a paused flip is refused', async () => {
    const { source, fabric } = await fresh();
    const todo = fabric.records.find((h) => h.record.state === 'reminder')!;
    await fabric.suspend(todo.path, '2099-01-01');
    expect(parseRecord(await source.readRecord(todo.path))).toMatchObject({ state: 'reminder', suspended: true, suspend_until: '2099-01-01' });
    expect(fabric.countsFor(folderOf(todo.path)).suspended).toBe(1);
    await fabric.unsuspend(todo.path);
    expect(parseRecord(await source.readRecord(todo.path))).toMatchObject({ suspended: false, suspend_until: null });
    const text = await source.readRecord(todo.path);
    await expect(source.writeRecord(todo.path, serializeRecord({ ...parseRecord(text), paused: true }), parseRecord(text).updated_at)).rejects.toThrow(/400/);
  });
});

describe('mark done before the agent finishes', () => {
  it('a tick on a working row writes only requests.complete, a todo row still writes state', async () => {
    const { source, fabric } = await fresh();
    const run = fabric.records.find((h) => h.record.state === 'in_progress')!;
    const writes = vi.spyOn(source, 'writeRecord');
    await fabric.complete(run);
    expect(writes).toHaveBeenCalledTimes(1);
    const sent = parseRecord(writes.mock.calls[0][1]);
    expect(sent).toMatchObject({ state: 'in_progress', step: run.record.step, requests: { complete: true } });
    expect({ ...sent, requests: run.record.requests, updated_at: '' }).toEqual({ ...run.record, schema: 7, updated_at: '', sections: sent.sections });
    await fabric.createTask(FOLDER, { title: 'Pay rent' });
    await fabric.complete(fabric.tasksFor(FOLDER).find((h) => h.record.title === 'Pay rent')!);
    expect(fabric.tasksFor(FOLDER).find((h) => h.record.title === 'Pay rent')!.record.state).toBe('succeeded');
  });

  it('schema 4 parses with complete false and serializes as schema 5', async () => {
    const { fabric } = await fresh();
    const text = serializeRecord(fabric.records[0].record).replace(/^schema: 7$/m, 'schema: 4').replace(', complete: false, redo: null', '');
    const r = parseRecord(text);
    expect(r.requests.complete).toBe(false);
    expect(serializeRecord(r)).toMatch(/^schema: 7$/m);
    expect(serializeRecord(r)).toContain('resume: false, complete: false, redo: null}');
  });

  it('is requestable on any open phase, not on done or failed, and shows the pending label', async () => {
    const { fabric } = await fresh();
    const open = fabric.records.filter((h) => !['succeeded', 'failed'].includes(h.record.state));
    for (const h of open) {
      expect(applyRequest(h.record, 'complete').requests.complete).toBe(true);
      expect(() => applyRequest(applyRequest(h.record, 'complete'), 'complete')).toThrow(/already/);
    }
    for (const h of fabric.records.filter((x) => ['succeeded', 'failed'].includes(x.record.state))) {
      expect(() => applyRequest(h.record, 'complete')).toThrow(/does not apply/);
      expect(canComplete(h.record, fabric.phaseFor(h))).toBe(false);
    }
    const run = fabric.records.find((h) => h.record.state === 'in_progress')!;
    expect(canComplete(run.record, fabric.phaseFor(run))).toBe(true);
    const asked = applyRequest(run.record, 'complete');
    expect(canComplete(asked, 'working')).toBe(false);
    const body = render(RecordScreen, { props: { record: asked, name: 'x' } }).body;
    expect(body).toContain(COMPLETE_PENDING);
    expect(body).not.toContain('data-testid="mark-done"');
    const idle = render(RecordScreen, { props: { record: run.record, name: 'x' } }).body;
    expect(idle).toContain('data-testid="mark-done"');
    const fin = fabric.records.find((h) => h.record.state === 'succeeded')!;
    expect(render(RecordScreen, { props: { record: fin.record, name: 'x' } }).body).not.toContain('data-testid="mark-done"');
  });
});
