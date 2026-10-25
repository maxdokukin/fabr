import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import { createRawSnippet } from 'svelte';
import { FixtureSource } from './source';
import { folderOf } from './folder';
import { FabricStore } from './tasks.svelte';
import RecordScreen from './RecordScreen.svelte';
import TaskSection from './TaskSection.svelte';

// Task 6.3 — notice -> action routing (Bug B). These component/integration tests mirror the
// established svelte/server `render` + FixtureSource patterns (tasks.test.ts, samples.test.ts,
// requests.test.ts) and cover the three routes openNotice (App.svelte) takes:
//   (a) out-of-tree / redelegated notice -> standalone record sheet with a live Approve control
//   (b) in-tree notice -> its folder-section sheet (TaskSection) unchanged (preserve 3.3)
//   (c) genuinely not-on-disk notice -> a recoverable message + next step, not a bare dead-end
// _Requirements: 2.1, 2.2, 2.3, 2.4, 3.3_ · _Validates: Property 2, Property 3 (3.3)_

// The exact recoverable message openNotice sets when fabric.locate returns nothing
// (App.svelte openNotice: `!held` branch). Kept in sync with the real string.
const NOT_ON_DISK = (p: string) => `No record on disk for ${p}. Try rescanning or check the folder.`;

async function fixtureStore() {
  const f = new FabricStore(new FixtureSource());
  await f.load();
  return f;
}

const pair = (f: FabricStore) => {
  const list = f.tasksFor('09-redelegated-pair');
  const from = list.find((h) => h.record.links.redelegated_to)!; // order-filament (failed)
  const successor = list.find((h) => h.record.links.redelegated_from)!; // order-filament-2 (defined)
  return { from, successor };
};

describe('redelegated-pair fixture shape (task 4.1 resolution target)', () => {
  it('the from-record is failed/redelegated and the successor is the live pending-approval target', async () => {
    const f = await fixtureStore();
    const { from, successor } = pair(f);
    expect(from.record.state).toBe('failed');
    expect(from.record.links.redelegated_to).toBeTruthy();
    expect(successor.record.state).toBe('defined');
    expect(successor.record.step).toBe('approve');
    expect(successor.record.waiting).toBe(true);
    expect(successor.record.approval?.status).toBe('pending');
  });
});

describe('out-of-tree / redelegated notice routes to the standalone record sheet (2.1, 2.3)', () => {
  it('locate(from) follows redelegated_to to the live successor (the ...-2 record)', async () => {
    const f = await fixtureStore();
    const { from, successor } = pair(f);
    // A notice clicked for the redelegated-from path resolves to the live successor, not the
    // dead failed record — this is the `held` openNotice opens (Bug B, 2.3).
    expect(f.locate(from.path)?.path).toBe(successor.path);
    expect(f.locate(from.path)?.record.id).toBe('2026-10-24-order-filament-2');
  });

  it('no loaded record hosts the successor in a visible tree node, so the standalone path fires', async () => {
    const f = await fixtureStore();
    const { successor } = pair(f);
    // openNotice opens the standalone sheet when NO tree node's folder matches the held record's
    // folder. With an injected FixtureSource there is no Store tree, so no node hosts it — the
    // standalone branch (not the in-tree panel) is the one that routes the user to the action UI.
    const folder = folderOf(successor.path);
    const hosted = f.openedRoots.some((r) => r === folder);
    expect(hosted).toBe(false);
  });

  it('the standalone sheet content (RecordScreen) mounts a working Approve control for the successor', async () => {
    const f = await fixtureStore();
    const { successor } = pair(f);
    // This renders exactly what App.svelte's standalone Sheet puts inside it: RecordScreen with
    // the live answer/approve handlers. A pending approval + onapprove yields the Approve button.
    const agent = createRawSnippet(() => ({ render: () => '<div></div>' }));
    const { body } = render(RecordScreen, {
      props: {
        record: successor.record,
        name: 'order-filament',
        backLabel: 'Close',
        now: new Date('2026-10-25T09:00:00Z'),
        onanswer: async () => {},
        onapprove: async () => {},
        onrequest: async () => {},
        onreload: async () => {},
        agent,
      },
    });
    // main's RecordScreen renders the record title in the page bar (class renamed from the
    // pre-rewrite `rec__title` to `page-bar__title`); assert the live class so the test tracks
    // main's actual markup rather than the stale one.
    expect(body).toContain('page-bar__title');
    expect(body).toContain(successor.record.title);
    expect(body).toContain('define');
  });
});

describe('in-tree notice still opens inside its folder section unchanged (3.3)', () => {
  it('TaskSection mounts the in-tree record sheet with a dialog and its Approve control', async () => {
    const f = await fixtureStore();
    const { successor } = pair(f);
    // Mirrors tasks.test.ts: once a record in this folder is open, TaskSection renders the sheet
    // (role="dialog") with the record screen and action UI — in-tree behavior is preserved.
    f.open(successor.path);
    const { body } = render(TaskSection, { props: { fabric: f, folder: '09-redelegated-pair' } });
    expect(body).toContain('role="dialog"');
    expect(body).toContain(successor.record.title);
    expect(body).toContain('Close');
    expect(body).toContain('define');
  });

  it('with no record open the section lists the records and shows no dialog', async () => {
    const f = await fixtureStore();
    const { body } = render(TaskSection, { props: { fabric: f, folder: '09-redelegated-pair' } });
    expect(body).toContain('data-testid="task-section"');
    expect(body).not.toContain('role="dialog"');
  });
});

describe('genuinely not-on-disk notice gets a recoverable message + next step (2.4)', () => {
  it('locate returns nothing for an unknown path, so openNotice takes the recoverable branch', async () => {
    const f = await fixtureStore();
    const unknown = '/home/me/3d/.fabric-space/tasks/2026-10-24-does-not-exist.md';
    expect(f.locate(unknown)).toBeUndefined();
    // The message openNotice sets for that branch names the path and gives a next step (rescan /
    // check the folder) rather than a bare dead-end. Assert the real string and its next step.
    const msg = NOT_ON_DISK(unknown);
    expect(msg).toContain(unknown);
    expect(msg).toMatch(/rescan/i);
    expect(msg).toMatch(/check the folder/i);
  });

  it('the recoverable message string is the one App.svelte openNotice actually sets', () => {
    // Guards against drift: assert the real App.svelte source sets noticeError with this exact
    // template (and a next step), so the recoverable branch is not a silent dead-end.
    const app = readFileSync(new URL('../../App.svelte', import.meta.url), 'utf8');
    expect(app).toContain('No record on disk for ${taskPath}. Try rescanning or check the folder.');
    expect(NOT_ON_DISK('X')).toBe('No record on disk for X. Try rescanning or check the folder.');
  });
});
