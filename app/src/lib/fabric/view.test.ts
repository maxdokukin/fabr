import { describe, expect, it } from 'vitest';
import { parseRecord } from './parse';
import { FixtureSource } from './source';
import type { Held } from './tasks.svelte';
import { findByTaskPath } from './view';

// Unit tests for findByTaskPath (task 4.1): resolution order is
//   (1) trailing-slash-normalized exact match
//   (2) normalized-path match (collapse `//`, trim trailing `/`)
//   (3) basename-id tie-break preferring state === 'in_progress'
//   (4) follow links.redelegated_to to the furthest live successor (cycle-guarded)
// A record with null/absent redelegated_to resolves exactly as (1)-(3) (preserve 3.6).
//
// Feature: agent-workspace-scope-and-notice-routing, Property 2
// Validates: Requirements 2.2, 2.3, 3.6

// Minimal shape accepted by findByTaskPath's `Resolvable` type parameter.
type Row = { path: string; record: { id: string; state: string; links: { redelegated_to: string | null } } };

const row = (
  path: string,
  id: string,
  state: string,
  redelegated_to: string | null = null,
): Row => ({ path, record: { id, state, links: { redelegated_to } } });

describe('findByTaskPath resolution order', () => {
  it('returns the exact path match ahead of any fallback', () => {
    const held = [
      row('/a/.fabric-space/tasks/t.md', 't', 'defined'),
      row('/b/.fabric-space/tasks/t.md', 't', 'in_progress'),
    ];
    expect(findByTaskPath(held, '/a/.fabric-space/tasks/t.md')?.path).toBe('/a/.fabric-space/tasks/t.md');
  });

  it('matches a normalized form with duplicate slashes', () => {
    const held = [row('/a/.fabric-space/tasks/t.md', 't', 'defined')];
    const resolved = findByTaskPath(held, '/a//.fabric-space///tasks/t.md');
    expect(resolved?.path).toBe('/a/.fabric-space/tasks/t.md');
  });

  it('matches a normalized form with a trailing slash', () => {
    const held = [row('/a/.fabric-space/tasks/t.md', 't', 'defined')];
    const resolved = findByTaskPath(held, '/a/.fabric-space/tasks/t.md/');
    expect(resolved?.path).toBe('/a/.fabric-space/tasks/t.md');
  });

  it('breaks a basename-id tie by preferring the in_progress record', () => {
    const held = [
      row('/stale/.fabric-space/tasks/order.md', 'order', 'failed'),
      row('/live/.fabric-space/tasks/order.md', 'order', 'in_progress'),
      row('/other/.fabric-space/tasks/order.md', 'order', 'defined'),
    ];
    // A path that matches none exactly but shares the basename id "order".
    const resolved = findByTaskPath(held, '/home/me/.fabric-space/tasks/order.md');
    expect(resolved?.record.state).toBe('in_progress');
    expect(resolved?.path).toBe('/live/.fabric-space/tasks/order.md');
  });

  it('falls back to the first basename-id match when none is in_progress', () => {
    const held = [
      row('/a/.fabric-space/tasks/order.md', 'order', 'failed'),
      row('/b/.fabric-space/tasks/order.md', 'order', 'defined'),
    ];
    const resolved = findByTaskPath(held, '/elsewhere/.fabric-space/tasks/order.md');
    expect(resolved?.path).toBe('/a/.fabric-space/tasks/order.md');
  });

  it('returns undefined when nothing matches', () => {
    const held = [row('/a/.fabric-space/tasks/one.md', 'one', 'defined')];
    expect(findByTaskPath(held, '/a/.fabric-space/tasks/two.md')).toBeUndefined();
  });
});

describe('findByTaskPath redelegation chase', () => {
  it('follows redelegated_to to the live successor by basename id', () => {
    const from = row('/x/.fabric-space/tasks/order.md', 'order', 'failed', '/y/.fabric-space/tasks/order-2.md');
    const to = row('/y/.fabric-space/tasks/order-2.md', 'order-2', 'defined');
    const resolved = findByTaskPath([from, to], '/x/.fabric-space/tasks/order.md');
    expect(resolved?.path).toBe('/y/.fabric-space/tasks/order-2.md');
    expect(resolved?.record.state).toBe('defined');
  });

  it('chases a multi-hop chain to the furthest live successor', () => {
    const a = row('/f/tasks/a.md', 'a', 'failed', '/f/tasks/b.md');
    const b = row('/f/tasks/b.md', 'b', 'failed', '/f/tasks/c.md');
    const c = row('/f/tasks/c.md', 'c', 'in_progress');
    const resolved = findByTaskPath([a, b, c], '/f/tasks/a.md');
    expect(resolved?.path).toBe('/f/tasks/c.md');
  });

  it('stops at the furthest loaded record when the next link is not loaded', () => {
    const a = row('/f/tasks/a.md', 'a', 'failed', '/f/tasks/b.md');
    const b = row('/f/tasks/b.md', 'b', 'failed', '/f/tasks/missing.md');
    const resolved = findByTaskPath([a, b], '/f/tasks/a.md');
    expect(resolved?.path).toBe('/f/tasks/b.md');
  });

  it('terminates on a cycle-guarded chain without looping forever', () => {
    const a = row('/f/tasks/a.md', 'a', 'failed', '/f/tasks/b.md');
    const b = row('/f/tasks/b.md', 'b', 'failed', '/f/tasks/a.md');
    const resolved = findByTaskPath([a, b], '/f/tasks/a.md');
    // The visited-set guard halts once a successor is revisited; returns a loaded record.
    expect(resolved).toBeDefined();
    expect([a.path, b.path]).toContain(resolved?.path);
  });

  it('leaves a null-links record resolving by its own path (preserve 3.6)', () => {
    const solo = row('/f/tasks/solo.md', 'solo', 'defined', null);
    const resolved = findByTaskPath([solo], '/f/tasks/solo.md');
    expect(resolved?.path).toBe('/f/tasks/solo.md');
  });
});

// Re-run of the task 1.2 exploratory assertion against the real fixture, now
// confirming the successor IS reached after the task 4.1 fix.
const raw = await new FixtureSource().listRecords();
const fixtureHeld: Held[] = raw.map(({ path, text }) => ({ path, record: parseRecord(text), text, source: null }));
const inPair = (h: Held) => h.path.includes('/09-redelegated-pair/');
const toHeld = fixtureHeld.find((h) => inPair(h) && h.path.endsWith('2026-10-24-order-filament-2.md'))!;
const fromHeld = fixtureHeld.find((h) => inPair(h) && h.path.endsWith('2026-10-24-order-filament.md'))!;

describe('findByTaskPath reaches the redelegated successor (09-redelegated-pair fixture)', () => {
  it('resolves the redelegated-from path to the live ...-2 successor', () => {
    const resolved = findByTaskPath(fixtureHeld, fromHeld.path);
    expect(resolved?.path).toBe(toHeld.path);
    expect(resolved?.record.state).toBe('defined');
    expect(resolved?.record.step).toBe('approve');
    expect(resolved?.record.waiting).toBe(true);
    expect(resolved?.record.approval?.status).toBe('pending');
  });
});
