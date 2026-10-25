import { describe, expect, it } from 'vitest';
import type { FabricEvent } from './events';
import { newRecord } from './source';
import type { FabricSource } from './source';
import type { TaskRecord } from './types';
import type { RegistryEntry } from './agents';
import { FabricStore, type Held, under } from './tasks.svelte';

// A loaded record at an absolute folder. `over` tweaks state/waiting/approval/links so each
// test can describe the record it needs without a fixture round-trip. The store accepts a null
// source and its $state fields are assignable directly (the pattern task 5.3 calls for).
function held(folder: string, id: string, over: Partial<TaskRecord> = {}): Held {
  const record: TaskRecord = { ...newRecord(id, { title: id }), ...over };
  const path = `${folder}/.fabric-space/tasks/${id}.md`;
  return { path, record, text: '', source: null };
}

function entry(taskPath: string): RegistryEntry {
  return {
    task_path: taskPath,
    task_id: taskPath.split('/').pop()!.replace(/\.md$/, ''),
    folder: '',
    step: 'run',
    started_at: '2026-10-24T18:00:00Z',
    heartbeat_at: '2026-10-24T18:00:00Z',
    heartbeat_interval_s: 10,
    cost_usd: 0,
    log_tail: [],
    question: null,
    pid: 1,
  };
}

function event(taskPath: string, kind: FabricEvent['kind'], at: string): FabricEvent {
  return { at, kind, taskPath, text: '' };
}

describe('under (path-segment containment)', () => {
  it('treats a folder as under itself and its descendants', () => {
    expect(under('/home/me/proj', '/home/me/proj')).toBe(true);
    expect(under('/home/me/proj', '/home/me/proj/sub/deep')).toBe(true);
    expect(under('/home/me/proj/', '/home/me/proj')).toBe(true);
  });
  it('does not treat a sibling with a shared prefix as under', () => {
    expect(under('/home/me/proj', '/home/me/project')).toBe(false);
    expect(under('/home/me/proj', '/home/me')).toBe(false);
  });
});

describe('FabricStore.scope derivation', () => {
  it('has no out-of-scope records and nothing to compare when no root is opened', () => {
    const f = new FabricStore(null);
    f.records = [held('/home/me/proj', 'a'), held('/home/me/elsewhere', 'b')];
    // openedRoots defaults to [] — an injected/test run captures no scope.
    expect(f.scope.workerRoot).toBeNull();
    expect(f.scope.openedRoots).toEqual([]);
    expect(f.scope.outOfScope).toEqual([]);
  });

  it('is empty when every loaded record is under an opened root (matched)', () => {
    const f = new FabricStore(null);
    f.workerRoot = '/home/me/proj';
    f.openedRoots = ['/home/me/proj'];
    f.records = [held('/home/me/proj', 'a'), held('/home/me/proj/sub', 'b')];
    expect(f.scope.workerRoot).toBe('/home/me/proj');
    expect(f.scope.outOfScope).toEqual([]);
  });

  it('surfaces records outside every opened root as out of scope (mismatched)', () => {
    const f = new FabricStore(null);
    f.workerRoot = '/home/me';
    f.openedRoots = ['/home/me/proj'];
    const inScope = held('/home/me/proj', 'a');
    const outA = held('/home/me/other', 'b');
    const outB = held('/home/me/fixtures/09', 'c');
    f.records = [inScope, outA, outB];
    expect(f.scope.outOfScope.map((h) => h.record.id)).toEqual(['b', 'c']);
    expect(f.scope.outOfScope).not.toContain(inScope);
  });

  it('counts a record in scope when any opened root contains it', () => {
    const f = new FabricStore(null);
    f.openedRoots = ['/a/one', '/a/two'];
    f.records = [held('/a/two/deep', 'a'), held('/a/three', 'b')];
    expect(f.scope.outOfScope.map((h) => h.record.id)).toEqual(['b']);
  });
});

describe('FabricStore.presenceElsewhere derivation', () => {
  it('is false with no opened root (nothing to compare)', () => {
    const f = new FabricStore(null);
    f.registry = [entry('/home/me/elsewhere/.fabric-space/tasks/x.md')];
    expect(f.presenceElsewhere).toBe(false);
  });

  it('is false when every registry entry drives a record under an opened root (matched)', () => {
    const f = new FabricStore(null);
    f.openedRoots = ['/home/me/proj'];
    const rec = held('/home/me/proj', 'x');
    f.records = [rec];
    f.registry = [entry(rec.path)];
    expect(f.presenceElsewhere).toBe(false);
  });

  it('is true when a registry entry drives a record outside the opened root (presence elsewhere)', () => {
    const f = new FabricStore(null);
    f.openedRoots = ['/home/me/proj'];
    f.records = [held('/home/me/proj', 'x')];
    // The worker's presence drives a record under ~/.fabric-space, not the opened project.
    f.registry = [entry('/home/me/.fabric-space/tasks/stray.md')];
    expect(f.presenceElsewhere).toBe(true);
  });
});

describe('FabricStore.markAllRead guard', () => {
  // events are newest-first; readPosition counts read events from the newest end.
  const waitingRec = () => held('/p', 'waiting-task', { waiting: true });
  const pendingRec = () =>
    held('/p', 'pending-task', { approval: { status: 'pending', by: null, at: null, note: null, digest: 'd' } });
  const doneRec = () => held('/p', 'done-task', { state: 'succeeded', waiting: false });

  it('marks everything read when no unread event is still actionable', () => {
    const f = new FabricStore(null);
    f.records = [doneRec()];
    f.events = [event('/p/.fabric-space/tasks/done-task.md', 'done', '2026-10-24T18:00:00Z')];
    f.readPosition = 0;
    expect(f.unread).toBe(1);
    f.markAllRead();
    expect(f.readPosition).toBe(1);
    expect(f.unread).toBe(0);
  });

  it('keeps a waiting notice reachable while advancing past newer resolved ones', () => {
    const f = new FabricStore(null);
    f.records = [waitingRec(), doneRec()];
    // newest-first: a newer resolved 'done', then an older still-waiting event.
    f.events = [
      event('/p/.fabric-space/tasks/done-task.md', 'done', '2026-10-24T18:05:00Z'),
      event('/p/.fabric-space/tasks/waiting-task.md', 'waiting', '2026-10-24T18:00:00Z'),
    ];
    f.readPosition = 0;
    expect(f.unread).toBe(2);
    f.markAllRead();
    // The oldest actionable event (index 1) is pinned unread; everything newer than it stays
    // unread too, so the waiting notice remains reachable.
    expect(f.readPosition).toBe(0);
    expect(f.unread).toBe(2);
  });

  it('keeps a pending-approval notice reachable', () => {
    const f = new FabricStore(null);
    f.records = [pendingRec()];
    f.events = [event('/p/.fabric-space/tasks/pending-task.md', 'waiting', '2026-10-24T18:00:00Z')];
    f.readPosition = 0;
    f.markAllRead();
    expect(f.unread).toBe(1);
    expect(f.readPosition).toBe(0);
  });

  it('advances past a resolved notice that is newer than the oldest actionable one only up to that pin', () => {
    const f = new FabricStore(null);
    f.records = [waitingRec(), doneRec()];
    // newest-first: resolved 'done' (newest), waiting (middle), another resolved 'done' (oldest).
    f.events = [
      event('/p/.fabric-space/tasks/done-task.md', 'done', '2026-10-24T18:06:00Z'),
      event('/p/.fabric-space/tasks/waiting-task.md', 'waiting', '2026-10-24T18:03:00Z'),
      event('/p/.fabric-space/tasks/done-task.md', 'done', '2026-10-24T18:00:00Z'),
    ];
    f.readPosition = 0;
    expect(f.unread).toBe(3);
    f.markAllRead();
    // Oldest actionable is index 1; readPosition = length - (1 + 1) = 1, so only the oldest
    // resolved event (index 2) is marked read; the waiting notice and newer events stay unread.
    expect(f.readPosition).toBe(1);
    expect(f.unread).toBe(2);
  });
});

// 3.4 one-writer-per-field: the GUI's only record writes come from answer/approval/request
// (writeRecord) and task creation (createRecord). The scope visibility additions (task 5.1) and
// the mark-read guard (task 5.2) are read-only — they must NOT write any record. This source
// throws on every mutating call, so if `markAllRead` or the scope/presence derivations ever
// reached a write the test would fail loudly.
describe('FabricStore scope/mark-read make no record writes (3.4)', () => {
  function noWriteSource(): FabricSource & { writes: number } {
    const src = {
      writes: 0,
      async listRecords() { return []; },
      async readRecord() { return ''; },
      async writeRecord() { this.writes++; throw new Error('3.4 violated: writeRecord called'); },
      async createRecord(): Promise<{ path: string; id: string }> {
        this.writes++; throw new Error('3.4 violated: createRecord called');
      },
      async listRegistry() { return []; },
      async readEvents() { return []; },
      eventsKey() { return 'k'; },
      async readBackendHeartbeat() { return null; },
      async stamp() { return '0'; },
    };
    return src as unknown as FabricSource & { writes: number };
  }

  it('markAllRead only moves readPosition and writes no record', () => {
    const src = noWriteSource();
    const f = new FabricStore(src);
    f.records = [held('/p', 'done-task', { state: 'succeeded', waiting: false })];
    f.events = [event('/p/.fabric-space/tasks/done-task.md', 'done', '2026-10-24T18:00:00Z')];
    f.readPosition = 0;
    const before = { records: f.records, errors: f.errors };
    f.markAllRead();
    expect(src.writes).toBe(0);
    expect(f.readPosition).toBe(1);
    // mark-read touches only read-position state: the loaded records are left untouched.
    expect(f.records).toBe(before.records);
    expect(f.errors).toBe(before.errors);
  });

  it('reading scope and presenceElsewhere derivations writes no record', () => {
    const src = noWriteSource();
    const f = new FabricStore(src);
    f.workerRoot = '/home/me';
    f.openedRoots = ['/home/me/proj'];
    f.records = [held('/home/me/proj', 'a'), held('/home/me/other', 'b')];
    f.registry = [
      { task_path: '/home/me/.fabric-space/tasks/stray.md', task_id: 'stray', folder: '', step: 'run',
        started_at: '2026-10-24T18:00:00Z', heartbeat_at: '2026-10-24T18:00:00Z', heartbeat_interval_s: 10,
        cost_usd: 0, log_tail: [], question: null, pid: 1 } as RegistryEntry,
    ];
    // Exercising the derivations must not mutate records or reach a write.
    expect(f.scope.outOfScope.map((h) => h.record.id)).toEqual(['b']);
    expect(f.presenceElsewhere).toBe(true);
    expect(src.writes).toBe(0);
  });
});
