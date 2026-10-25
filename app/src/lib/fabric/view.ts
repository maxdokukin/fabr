import { parseRegistryEntry, type RegistryEntry } from './agents';
import type { RawRecord } from './source';

export function loadRegistry(raw: RawRecord[]): { entries: RegistryEntry[]; unreadable: number } {
  const byPath = new Map<string, RegistryEntry>();
  let unreadable = 0;
  for (const { text } of raw) {
    try {
      const e = parseRegistryEntry(text);
      const prev = byPath.get(e.task_path);
      if (!prev || Date.parse(e.heartbeat_at) > Date.parse(prev.heartbeat_at)) byPath.set(e.task_path, e);
    } catch {
      unreadable += 1;
    }
  }
  return { entries: [...byPath.values()], unreadable };
}

// Normalize a path for comparison: collapse duplicate slashes and trim a trailing slash.
// The browser cannot call realpath, so this string normalization (plus the basename-id
// fallback below) is the frontend's "canonical" step; the authoritative key stays the
// backend's task_path_key.
const normPath = (p: string) => p.replace(/\/{2,}/g, '/').replace(/(.)\/+$/, '$1');

const base = (p: string) => (normPath(p).split('/').pop() ?? p).replace(/\.md$/, '');

type Resolvable = { path: string; record: { id: string; state: string; links: { redelegated_to: string | null } } };

// Resolve a loaded record by (1) trailing-slash-normalized exact match, (2) normalized-path
// match, (3) basename-id tie-break preferring in_progress (mock paths differ from registry
// paths). Returns undefined when nothing matches.
function resolveBase<T extends Resolvable>(held: T[], taskPath: string): T | undefined {
  const exact = held.find((h) => h.path === taskPath);
  if (exact) return exact;
  const target = normPath(taskPath);
  const normed = held.find((h) => normPath(h.path) === target);
  if (normed) return normed;
  const id = base(taskPath);
  if (!id) return undefined;
  const same = held.filter((h) => h.record.id === id);
  return same.find((h) => h.record.state === 'in_progress') ?? same[0];
}

// Follow links.redelegated_to transitively to the furthest loaded successor, matching each
// link by normalized path or basename-id. A visited-set guards against cycles. When a link
// points at a successor that is not loaded, the furthest record still loaded is returned.
function followRedelegation<T extends Resolvable>(held: T[], start: T): T {
  const visited = new Set<string>([start.path]);
  let current = start;
  for (;;) {
    const next = current.record.links.redelegated_to;
    if (!next) return current;
    const successor = resolveBase(held, next);
    if (!successor || visited.has(successor.path)) return current;
    visited.add(successor.path);
    current = successor;
  }
}

// Resolve a notice taskPath to a loaded record. Resolution order:
//   (1) trailing-slash-normalized exact match; (2) normalized-path match;
//   (3) basename-id tie-break preferring in_progress;
//   (4) follow links.redelegated_to to the furthest live successor (cycle-guarded).
// A record with null/absent redelegated_to resolves exactly as in (1)-(3).
export function findByTaskPath<T extends Resolvable>(held: T[], taskPath: string): T | undefined {
  const resolved = resolveBase(held, taskPath);
  if (!resolved) return undefined;
  return followRedelegation(held, resolved);
}

// Whether two task paths denote the same record, using the same tolerance findByTaskPath
// applies (exact, then slash-normalized, then basename id). Lets callers correlate a notice's
// taskPath with an entry keyed by its on-disk path without re-implementing normalization.
export function samePath(a: string, b: string): boolean {
  return a === b || normPath(a) === normPath(b) || (base(a) !== '' && base(a) === base(b));
}
