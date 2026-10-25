import { describe, expect, it } from 'vitest';
import type { FabricSource } from './source';
import { FabricStore } from './tasks.svelte';
import { samePath } from './view';

// When a record on disk fails to parse, the store must surface it as "on disk but unreadable"
// (via unreadableReason) rather than silently dropping it, so a notice can give an accurate,
// recoverable message instead of a misleading "No record on disk" dead-end. (This mirrors the
// field case where a record was written with a malformed question.answer; the frontend parser
// checks field presence + state/step validity, so here we use an invalid `state` to trigger a
// real parseRecord failure the frontend detects.)
const FM = (id: string, state: string) => `---
schema: 2
id: ${id}
source: null
title: write email
due: null
state: ${state}
step: define
waiting: true
created_at: 2026-10-25T08:25:13Z
updated_at: 2026-10-25T08:26:35Z
requests: { delegate: false, cancel: false, redelegate: false }
question: { id: q1, kind: open, text: 'ask', options: null, prior: null, fields: null, asked_at: 2026-10-25T08:26:35Z, answer: null }
answers: []
grants: { requested: [], granted: [] }
plan_meta: null
assertions: []
approval: null
verdict: null
cost_usd: 0.0
links: { redelegated_from: null, redelegated_to: null }
---
## Context
From the todo list.
## Spec
## Success
## Plan
## Log
`;

const DIR = '/Users/me/fabr-workspace/.fabric-space/tasks';
const VALID_PATH = `${DIR}/2026-10-25-write-email.md`;
const CORRUPT_PATH = `${DIR}/2026-10-25-write-email-2.md`;

function source(): FabricSource {
  return {
    listRecords: async () => [
      { path: VALID_PATH, text: FM('2026-10-25-write-email', 'task') },
      { path: CORRUPT_PATH, text: FM('2026-10-25-write-email-2', 'not-a-real-state') },
    ],
    listRegistry: async () => [],
    readEvents: async () => [],
    eventsKey: () => 'k',
    readBackendHeartbeat: async () => null,
    stamp: async () => '1',
    readRecord: async () => '',
    writeRecord: async () => {},
    createRecord: async () => ({ path: '' }),
  } as unknown as FabricSource;
}

describe('samePath tolerance', () => {
  it('matches exact, slash-normalized, and basename-id forms', () => {
    expect(samePath(CORRUPT_PATH, CORRUPT_PATH)).toBe(true);
    expect(samePath(CORRUPT_PATH, `${CORRUPT_PATH}/`)).toBe(true);
    expect(samePath(CORRUPT_PATH.replace('/tasks/', '/tasks//'), CORRUPT_PATH)).toBe(true);
    expect(samePath('/other/tasks/2026-10-25-write-email-2.md', CORRUPT_PATH)).toBe(true);
  });
  it('does not match different records', () => {
    expect(samePath(VALID_PATH, CORRUPT_PATH)).toBe(false);
  });
});

describe('FabricStore unreadable-record diagnostics', () => {
  it('loads the valid record and tracks the corrupt one as unreadable', async () => {
    const f = new FabricStore(source());
    await f.load();
    // The valid record is loaded and locatable.
    expect(f.records.map((h) => h.path)).toEqual([VALID_PATH]);
    expect(f.locate(VALID_PATH)?.record.id).toBe('2026-10-25-write-email');
    // The corrupt record is NOT in records (parse failed) but IS tracked as unreadable.
    expect(f.locate(CORRUPT_PATH)).toBeUndefined();
    expect(f.unreadable.has(CORRUPT_PATH)).toBe(true);
  });

  it('distinguishes an unreadable record from a genuinely missing one', async () => {
    const f = new FabricStore(source());
    await f.load();
    // On disk but failed to parse -> a reason is returned (names the invalid field/state).
    const reason = f.unreadableReason(CORRUPT_PATH);
    expect(reason).toBeTruthy();
    expect(reason).toMatch(/state/i);
    // Tolerant of path form (trailing slash).
    expect(f.unreadableReason(`${CORRUPT_PATH}/`)).toBeTruthy();
    // A path that is neither loaded nor unreadable -> null (genuinely missing).
    expect(f.unreadableReason(`${DIR}/does-not-exist.md`)).toBeNull();
    // A loaded, valid record is not "unreadable".
    expect(f.unreadableReason(VALID_PATH)).toBeNull();
  });
});
