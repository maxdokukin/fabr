import fc from 'fast-check';
import { describe, expect, it } from 'vitest';
import { findByTaskPath } from './view';

// Property-based tests for findByTaskPath (task 4.3).
//
// Feature: agent-workspace-scope-and-notice-routing, Property 2: Notice resolution —
// resolution returns the furthest live successor (or the recoverable branch when no live
// target), and returns exactly the original result for non-redelegated, matching-form inputs.
//
// Validates: Requirements 2.2, 2.3, 3.6

const NUM_RUNS = 200; // >= 100 per the design's property-test minimum

// Minimal shape accepted by findByTaskPath's `Resolvable` type parameter.
type Row = { path: string; record: { id: string; state: string; links: { redelegated_to: string | null } } };

const row = (
  path: string,
  id: string,
  state: string,
  redelegated_to: string | null = null,
): Row => ({ path, record: { id, state, links: { redelegated_to } } });

// A canonical record path for a unique basename id.
const pathFor = (id: string) => `/root/${id}/.fabric-space/tasks/${id}.md`;

// Re-apply the forms findByTaskPath treats as equivalent: inject duplicate slashes and/or
// append a trailing slash. The resolver normalizes `//` -> `/` and trims a trailing `/`, so
// every variant below denotes the same path (a "matching form").
function variantForm(canonical: string, dupSlashes: boolean, trailing: boolean): string {
  let p = canonical;
  if (dupSlashes) p = p.replace(/\//g, '//');
  if (trailing) p = p + '/';
  return p;
}

const stateArb = fc.constantFrom('defined', 'in_progress', 'planning', 'failed', 'done');

// A chain of records id_0 -> id_1 -> ... -> id_{n-1}. Each non-terminal record redelegates to
// the next; the terminal record has null links. `liveAt` marks how far the chain is actually
// loaded: records with index < liveAt are present in `held`, the rest are dangling links.
// Every id is unique (its own folder), so basename-id resolution is unambiguous.
const chainArb = fc
  .record({
    length: fc.integer({ min: 1, max: 6 }),
    liveCount: fc.integer({ min: 1, max: 6 }),
    states: fc.array(stateArb, { minLength: 6, maxLength: 6 }),
    dupSlashes: fc.boolean(),
    trailing: fc.boolean(),
    tag: fc.integer({ min: 0, max: 0xffffff }),
  })
  .map(({ length, liveCount, states, dupSlashes, trailing, tag }) => {
    const ids = Array.from({ length }, (_, i) => `t${tag.toString(16)}-${i}`);
    const live = Math.min(liveCount, length);
    const rows: Row[] = [];
    for (let i = 0; i < live; i++) {
      const next = i + 1 < length ? pathFor(ids[i + 1]) : null;
      rows.push(row(pathFor(ids[i]), ids[i], states[i], next));
    }
    const headCanonical = pathFor(ids[0]);
    return {
      held: rows,
      headCanonical,
      headQuery: variantForm(headCanonical, dupSlashes, trailing),
      live,
    };
  });

describe('Property 2: Notice resolution picks the furthest live successor', () => {
  it('returns the furthest loaded record in the redelegation chain (>= 100 runs)', () => {
    fc.assert(
      fc.property(chainArb, ({ held, headQuery, live }) => {
        const resolved = findByTaskPath(held, headQuery);
        // The furthest live successor is the last loaded record in the chain.
        const expected = held[live - 1];
        expect(resolved).toBeDefined();
        expect(resolved?.path).toBe(expected.path);
      }),
      { numRuns: NUM_RUNS },
    );
  });

  it('is invariant to equivalent path forms (duplicate slashes / trailing slash)', () => {
    fc.assert(
      fc.property(chainArb, ({ held, headCanonical, headQuery }) => {
        // Querying by any equivalent form resolves to the same target as the canonical form.
        const byCanonical = findByTaskPath(held, headCanonical);
        const byVariant = findByTaskPath(held, headQuery);
        expect(byVariant?.path).toBe(byCanonical?.path);
      }),
      { numRuns: NUM_RUNS },
    );
  });
});

// Preservation (3.6): a record with no redelegation link resolves by its own path, and a
// non-redelegated, matching-form query returns exactly the record it names. We model the
// original (pre-fix) behavior for these inputs as plain exact/normalized/basename resolution,
// which must coincide with the fixed resolver when no links are followed.
describe('Property 2: non-redelegated matching-form inputs are unchanged (preserve 3.6)', () => {
  // A set of records with null links and unique basename ids (no redelegation anywhere).
  const soloSetArb = fc
    .record({
      count: fc.integer({ min: 1, max: 6 }),
      states: fc.array(stateArb, { minLength: 6, maxLength: 6 }),
      dupSlashes: fc.boolean(),
      trailing: fc.boolean(),
      pick: fc.integer({ min: 0, max: 5 }),
      tag: fc.integer({ min: 0, max: 0xffffff }),
    })
    .map(({ count, states, dupSlashes, trailing, pick, tag }) => {
      const ids = Array.from({ length: count }, (_, i) => `s${tag.toString(16)}-${i}`);
      const held = ids.map((id, i) => row(pathFor(id), id, states[i], null));
      const target = held[pick % count];
      return {
        held,
        target,
        query: variantForm(target.path, dupSlashes, trailing),
      };
    });

  it('resolves a null-links record to exactly its own path (>= 100 runs)', () => {
    fc.assert(
      fc.property(soloSetArb, ({ held, target, query }) => {
        const resolved = findByTaskPath(held, query);
        // No links to follow: the fixed resolver returns exactly the named record, which is
        // identical to the original resolve-by-own-path behavior (3.6).
        expect(resolved?.path).toBe(target.path);
        expect(resolved?.record.links.redelegated_to).toBeNull();
      }),
      { numRuns: NUM_RUNS },
    );
  });
});
