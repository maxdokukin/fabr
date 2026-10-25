import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import type { Api } from './api/index';
import type { Node, Tree } from './api/types';
import { Store } from './store.svelte';
import Toolbar from './Toolbar.svelte';
import Notice from './Notice.svelte';
import NoticesList from './fabric/NoticesList.svelte';
import { FabricStore, type Held } from './fabric/tasks.svelte';
import { newRecord } from './fabric/source';
import type { TaskRecord } from './fabric/types';
import type { FabricEvent } from './fabric/events';

const today = new Date().toISOString().slice(0, 10);

// A loaded record under an absolute folder (mirrors scope.test.ts's helper). Lets each test
// describe a matched/mismatched workspace without a fixture round-trip.
function held(folder: string, id: string, over: Partial<TaskRecord> = {}): Held {
  const record: TaskRecord = { ...newRecord(id, { title: id }), ...over };
  return { path: `${folder}/.fabric-space/tasks/${id}.md`, record, text: '', source: null };
}

function event(taskPath: string, kind: FabricEvent['kind'] = 'waiting', at = '2026-10-24T18:00:00Z'): FabricEvent {
  return { at, kind, taskPath, text: 'needs you' };
}

// A fabric in MATCHED scope: the search root equals the single opened root and no record falls
// outside it, so scopeMismatch is false.
function matchedFabric(): FabricStore {
  const f = new FabricStore(null);
  f.workerRoot = '/home/me/proj';
  f.openedRoots = ['/home/me/proj'];
  f.records = [held('/home/me/proj', 'a'), held('/home/me/proj/sub', 'b')];
  return f;
}

// A fabric in MISMATCHED scope: the search root is broader than the opened root and a record
// lives outside it, so scopeMismatch is true.
function mismatchedFabric(): FabricStore {
  const f = new FabricStore(null);
  f.workerRoot = '/home/me';
  f.openedRoots = ['/home/me/proj'];
  f.records = [held('/home/me/proj', 'a'), held('/home/me/elsewhere', 'b')];
  return f;
}

// A minimal loaded Store so the real Toolbar renders (Crumbs/counts/search read displayRoot,
// today, view, query, multiSource). Mirrors addtask.test.ts's tree + getTree/getStamp wiring.
async function loadedStore(): Promise<Store> {
  const n = (path: string, o: Partial<Node> = {}): Node => ({
    source: 'local', path, name: path || 'root', managed: true, meta: {}, items: [], warnings: [], version: 'v', via: null, children: [], ...o,
  });
  const tree: Tree = {
    root: { ...n('', { managed: false, source: '' }), children: [n('', { children: [n('work')] })] },
    sources: [{ id: 'local', name: 'Home', host: null, root: '/tmp/x', prune: null }],
    today, truncated: false, errors: [],
  };
  const api = { getTree: vi.fn(async () => tree), getStamp: vi.fn(async () => ({ stamp: 's' })) } as unknown as Api;
  const store = new Store(api);
  await store.load();
  return store;
}

describe('Toolbar backend indicator — scope visibility (Req 1.1, 1.3)', () => {
  it('matched scope: no warning modifier and the title names the search root', async () => {
    const store = await loadedStore();
    const fabric = matchedFabric();
    const { body } = render(Toolbar, {
      props: { store, fabric, onrescan: () => {}, onexpand: () => {}, oncollapse: () => {}, onsearchenter: () => {} },
    });
    expect(body).not.toContain('backend--warn');
    expect(body).toContain('searching /home/me/proj');
  });

  it('mismatched root: warning modifier is present and the title still names the search root', async () => {
    const store = await loadedStore();
    const fabric = mismatchedFabric();
    const { body } = render(Toolbar, {
      props: { store, fabric, onrescan: () => {}, onexpand: () => {}, oncollapse: () => {}, onsearchenter: () => {} },
    });
    expect(body).toContain('backend--warn');
    expect(body).toContain('searching /home/me');
  });

  it('mismatched via an out-of-scope record under a matching root: warning modifier is present', async () => {
    const store = await loadedStore();
    const fabric = new FabricStore(null);
    fabric.workerRoot = '/home/me/proj';
    fabric.openedRoots = ['/home/me/proj'];
    // Root matches, but a loaded record sits outside the opened root → out of scope.
    fabric.records = [held('/home/me/proj', 'a'), held('/home/me/other', 'b')];
    const { body } = render(Toolbar, {
      props: { store, fabric, onrescan: () => {}, onexpand: () => {}, oncollapse: () => {}, onsearchenter: () => {} },
    });
    expect(body).toContain('backend--warn');
  });
});

describe('Notice banner — scope guidance (Req 1.1, 1.3)', () => {
  // Notice is presentational over store.tree?.errors / store.notice; a minimal stub is enough.
  const noticeStore = () => ({ tree: { errors: [] }, notice: null }) as unknown as import('./store.svelte').Store;

  it('matched scope: no scope banner is shown', () => {
    const { body } = render(Notice, { props: { store: noticeStore(), fabric: matchedFabric() } });
    expect(body).not.toContain('data-testid="scope-notice"');
  });

  it('mismatched scope: the actionable scope banner names the search root', () => {
    const { body } = render(Notice, { props: { store: noticeStore(), fabric: mismatchedFabric() } });
    expect(body).toContain('data-testid="scope-notice"');
    expect(body).toContain('Agent is searching /home/me');
    expect(body).toContain('Scope default_folder to this project');
  });

  it('shows no scope banner without a fabric', () => {
    const { body } = render(Notice, { props: { store: noticeStore(), fabric: null } });
    expect(body).not.toContain('data-testid="scope-notice"');
  });
});

describe('NoticesList out-of-scope tag (Req 1.2)', () => {
  const events: FabricEvent[] = [
    event('/home/me/proj/.fabric-space/tasks/in.md'),
    event('/home/me/other/.fabric-space/tasks/out.md'),
  ];

  it('marks only the notice whose resolved target the predicate flags out of scope', () => {
    const outOfScope = (p: string) => p.includes('/home/me/other/');
    const { body } = render(NoticesList, { props: { events, readPosition: 0, outOfScope } });
    expect(body).toContain('data-testid="oos-tag"');
    expect(body).toContain('out of scope');
    expect(body).toContain('nrow--oos');
    // Exactly one row carries the tag (the out-of-scope one).
    expect(body.match(/data-testid="oos-tag"/g)).toHaveLength(1);
  });

  it('renders no tag when the predicate flags nothing out of scope', () => {
    const { body } = render(NoticesList, { props: { events, readPosition: 0, outOfScope: () => false } });
    expect(body).not.toContain('data-testid="oos-tag"');
    expect(body).not.toContain('nrow--oos');
  });

  it('renders no tag when no predicate is provided', () => {
    const { body } = render(NoticesList, { props: { events, readPosition: 0 } });
    expect(body).not.toContain('data-testid="oos-tag"');
  });
});
