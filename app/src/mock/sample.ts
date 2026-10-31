import type { Item, Meta, Node, Pri } from '../lib/api/types';
import type { Phase } from '../lib/fabric/phase';
import { taskSlug } from '../lib/fabric/source';

type T = [Phase, number | null, string, Pri?, string?];
type Spec = [path: string, meta: Meta | null, todos: T[] | null];

export const SAMPLE: Spec[] = [
  ['', { name: 'fabr', kind: 'root', description: 'Personal workspace root', show_depth: 1 },
    [['working', 3, 'Define fabr spec v0', 1], ['todo', 14, 'Quarterly review of all nodes'], ['done', -5, 'Pick a name']]],
  ['work', { name: 'work', kind: 'area' },
    [['todo', 1, 'Submit timesheet', 2, '17:00'], ['todo', -2, 'Reply to vendor contract email', 1, '10:00']]],
  ['work/api-server', { name: 'api-server', kind: 'project', color: 'teal' },
    [['working', 2, 'Migrate auth to OAuth2', 1, '09:15'], ['todo', 10, 'Add rate limiting', 3], ['paused', 5, 'Backfill audit logs', 2], ['failed', -3, 'Rotate signing keys', 1], ['todo', null, 'Investigate flaky integration test', 2],
     ['done', -12, 'Set up CI pipeline'], ['done', -1, 'Fix N+1 query on /users']]],
  ['work/api-server/docs', { name: 'docs', kind: 'folder' },
    [['todo', 7, 'Write OpenAPI descriptions'], ['todo', 7, 'Add auth flow diagram']]],
  ['work/api-server/tests', { name: 'tests', kind: 'folder' }, []],
  ['work/frontend', { name: 'frontend', kind: 'project', show_depth: 1 },
    [['working', 0, 'Ship dark mode', 2, '23:30'], ['todo', 21, 'Upgrade to React 20'], ['suspended', 40, 'Evaluate Svelte 6 migration'], ['done', -30, 'Design system tokens']]],
  ['work/frontend/node_modules', null, null],
  ['work/meetings', { name: 'meetings', kind: 'folder' }, null],
  ['personal', { name: 'personal', kind: 'area' }, [['todo', 30, 'Renew passport']]],
  ['personal/health', { name: 'health', kind: 'folder' },
    [['todo', 5, 'Book dentist', undefined, '14:30'], ['working', null, 'Run 3x per week'], ['done', -3, 'Blood test']]],
  ['personal/finance', { name: 'finance', kind: 'folder', show_depth: 1 },
    [['todo', -10, 'File tax return', 1], ['todo', 60, 'Rebalance portfolio'], ['done', -40, 'Close old bank account']]],
  ['personal/finance/receipts', null, null],
  ['personal/finance/receipts/2026', { name: '2026', kind: 'folder' }, [['todo', 75, 'Scan Q4 receipts']]],
  ['research', { name: 'research', kind: 'area', color: 'pink' }, [['working', 9, 'Lit review: tree layout algorithms']]],
  ['research/paper-a', { name: 'paper-a', kind: 'project' },
    [['todo', 4, 'Address reviewer 2', 1], ['working', 4, 'Rerun ablations', 2], ['todo', 4, 'Update related work'], ['done', -20, 'Submit draft']]],
  ['research/paper-a/figures', { name: 'figures', kind: 'folder' }, [['todo', 3, 'Regenerate fig 3 at 300dpi']]],
  ['research/paper-a/figures/raw', { name: 'raw', kind: 'folder' }, [['done', -25, 'Export raw data']]],
  ['research/ideas', { name: 'ideas', kind: 'folder' },
    Array.from({ length: 12 }, (_, i): T => ['todo', null, `Idea #${i + 1}: something worth exploring`])],
  ['archive', { name: 'archive', kind: 'area', hidden: true },
    [['done', -200, 'Old project wrap-up'], ['done', -180, 'Hand off repo']]],
];

export function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export function addDays(today: string, n: number): string {
  const [y, m, d] = today.split('-').map(Number);
  return isoDay(new Date(y, m - 1, d + n));
}

function hex4(s: string): string {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return (h >>> 0).toString(16).padStart(8, '0').slice(0, 4);
}

const PRUNE = new Set(['node_modules', '.git', '.venv', 'venv', '__pycache__', 'target', 'dist', 'build']);

export function buildSample(source: string, today: string, specs: Spec[] = SAMPLE, showAll = false): Node {
  const nodes = new Map<string, Node>();
  for (const [path, meta, todos] of specs) {
    const name = path.split('/').pop() || source;
    const items: Item[] = (todos ?? []).map(([phase, off, title, pri, time]) => ({
      id: `${today}-${taskSlug(title)}`, phase, due: off === null ? null : addDays(today, off) + (time ? `T${time}` : ''), title, pri: pri ?? 0,
    }));
    nodes.set(path, {
      source, path, name, managed: meta !== null, meta: meta ?? {}, items, warnings: [],
      version: meta !== null ? hex4(path + 'v').repeat(10) : null, via: null, children: [],
    });
  }
  const root = nodes.get('')!;
  const parentPath = (p: string) => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : '');
  const emitted = (p: string): boolean => {
    const n = nodes.get(p);
    if (!n || p.split('/').some((s) => PRUNE.has(s))) return false;
    if (n.managed || !p || showAll) return true;
    let anc = parentPath(p);
    let d = 1;
    while (anc && !nodes.get(anc)?.managed) { anc = parentPath(anc); d++; }
    return d <= Number(nodes.get(anc)?.meta.show_depth ?? 0);
  };
  for (const [path, n] of nodes) {
    if (!path || !emitted(path)) continue;
    const skipped: string[] = [];
    let p = parentPath(path);
    while (p && !emitted(p)) { skipped.unshift(p.split('/').pop()!); p = parentPath(p); }
    if (skipped.length) n.via = [...skipped, n.name].join('/');
    nodes.get(p)!.children.push(n);
  }
  return root;
}

export const LAB: Spec[] = [
  ['', { name: 'lab', kind: 'server', show_depth: 0 }, [['working', 1, 'Rotate TLS certificates', 1], ['todo', 12, 'Upgrade kernel']]],
  ['backups', { name: 'backups', kind: 'service' }, [['todo', -1, 'Verify restore of last snapshot', 1], ['done', -6, 'Prune old snapshots']]],
  ['monitoring', { name: 'monitoring', kind: 'service' }, [['todo', 6, 'Alert on disk > 85%', 2]]],
  ['srv', null, null],
  ['srv/wiki', { name: 'wiki', kind: 'app' }, [['todo', 20, 'Migrate to static export']]],
];

export const VM: Spec[] = [
  ['', { name: 'sample', kind: 'server', show_depth: 1 }, [['todo', null, 'Check printer firmware'], ['working', null, 'Order filament']]],
  ['app', { name: 'app', kind: 'project' }, [['needs_you', 2, 'Order filament spools', 1], ['needs_harness', 3, 'Write release notes to disk (needs write_file)'], ['needs_you', 5, 'Approve the release plan']]],
];

export const VM_QA: Spec[] = [
  VM[0],
  ['app', { name: 'app', kind: 'project' }, [...VM[1][2]!, ['needs_you', 6, 'Choose the shipping address', 1], ['needs_you', 7, 'Re-plan the release', 1]]],
];
