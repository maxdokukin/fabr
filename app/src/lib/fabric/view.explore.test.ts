import { describe, expect, it } from 'vitest';
import { folderOf } from './folder';
import { parseRecord } from './parse';
import { FixtureSource } from './source';
import type { Held } from './tasks.svelte';
import { findByTaskPath } from './view';

// Task 1.2 exploratory bug-condition test for Bug B (notice -> approval routing),
// FLIPPED to the FIXED expectation after the task 4.1 rewrite of `findByTaskPath`.
//
// Feature: agent-workspace-scope-and-notice-routing, Property 2
// Validates: Requirements 2.1, 2.2, 2.3 (the FIX on the now-fixed resolver).
//
// On the unfixed code these assertions encoded the DEFECT: `findByTaskPath`
// (app/src/lib/fabric/view.ts) ignored `links.redelegated_to` and failed on a
// differing path form, so the live successor was never reached. Task 4.1 made
// the resolver normalize path forms and follow `redelegated_to` to the furthest
// live successor. The two counterexample assertions below now assert the fixed
// behavior: the redelegated-from path reaches the `...-2.md` successor, and a
// differing path form resolves. The routing-gate dead-end cases are addressed by
// the task 6 `openNotice` rewrite; the resolver layer here is now correct.

const raw = await new FixtureSource().listRecords();
const held: Held[] = raw.map(({ path, text }) => ({ path, record: parseRecord(text), text, source: null }));

// The 09-redelegated-pair fixture: a failed "from" record that links forward to
// a live "to" successor (state=defined, step=approve, waiting=true, approval=pending).
// Paths come from the glob, so discover the real loaded path forms here.
const inPair = (h: Held) => h.path.includes('/09-redelegated-pair/');
const toHeld = held.find((h) => inPair(h) && h.path.endsWith('2026-10-24-order-filament-2.md'))!;
const fromHeld = held.find((h) => inPair(h) && h.path.endsWith('2026-10-24-order-filament.md'))!;
const FROM_PATH = fromHeld.path;
const TO_PATH = toHeld.path;

describe('Bug B fixed: findByTaskPath reaches the live successor on FIXED code', () => {
  it('fixture precondition: the "from" record is failed and links to a live "to" successor', () => {
    expect(fromHeld.record.state).toBe('failed');
    expect(fromHeld.record.links.redelegated_to).toBe(
      '/home/me/3d/.fabric-space/tasks/2026-10-24-order-filament-2.md',
    );
    // The live successor is the actionable one the user needs to reach.
    expect(toHeld.record.state).toBe('defined');
    expect(toHeld.record.step).toBe('approve');
    expect(toHeld.record.waiting).toBe(true);
    expect(toHeld.record.approval?.status).toBe('pending');
  });

  // Requirement 2.3 (fixed): resolving the redelegated-FROM path now follows
  // links.redelegated_to to the live successor.
  it('reaches the ...-2 live successor when resolving the redelegated-from path (FIXED)', () => {
    const resolved = findByTaskPath(held, FROM_PATH);
    // Now resolves THROUGH the dead "from" record to its live successor.
    expect(resolved?.path).toBe(TO_PATH);
    expect(resolved?.path).not.toBe(FROM_PATH);
    expect(resolved?.record.state).toBe('defined');
    expect(resolved?.record.step).toBe('approve');
    // The on-disk link was followed by the fixed resolver.
    expect(fromHeld.record.links.redelegated_to).not.toBeNull();
  });

  // Requirement 2.2 (fixed): a differing path form (trailing slash) now resolves
  // via the normalized-path match to the loaded record.
  it('resolves a differing path form of a loaded record (FIXED)', () => {
    const differingForm = `${TO_PATH}/`; // trailing slash: normalized to the loaded path
    const resolved = findByTaskPath(held, differingForm);
    expect(resolved?.path).toBe(TO_PATH);
  });

  // Preservation check (3.6): a non-redelegated record still resolves by its own
  // path exactly as before.
  it('resolves a non-redelegated in-tree record by its own path (preserve 3.6)', () => {
    const resolved = findByTaskPath(held, TO_PATH);
    expect(resolved?.path).toBe(TO_PATH);
    expect(folderOf(resolved!.path)).toBe(folderOf(TO_PATH));
  });
});
