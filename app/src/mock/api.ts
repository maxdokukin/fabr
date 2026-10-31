import type { Api } from '../lib/api/index';
import {
  ApiError, type Meta, type Node, type NodeRef, type Settings, type Source, type TestResult, type Tree, type DirEntry, type FabricSettings, type FabricSecretsStatus, type NodeKnowledge, type KnowledgeKind, type Proposal, type Transcript,
} from '../lib/api/types';
import { FixtureSource, guardState, newRecord, taskSlug, updatedAtOf } from '../lib/fabric/source';
import { parseRecord } from '../lib/fabric/parse';
import type { CcSession } from '../lib/api/types';
import type { TaskRecord } from '../lib/fabric/types';
import { serializeRecord } from '../lib/fabric/serialize';
import { buildSample, isoDay, LAB, SAMPLE, VM, VM_QA } from './sample';

interface MockOpts {
  empty?: boolean;
  multi?: boolean;
  today?: string;
  latency?: number;
}

export function createMockApi(opts: MockOpts = {}): Api {
  const empty = opts.empty ?? (typeof location !== 'undefined' && new URLSearchParams(location.search).has('empty'));
  const multi = opts.multi ?? (typeof location !== 'undefined' && new URLSearchParams(location.search).has('multi'));
  const today = opts.today ?? isoDay(new Date());
  const latency = opts.latency ?? 60;
  let sources: Source[] = empty ? [] : [
    { id: 'local', name: 'Home', host: null, root: '~/fabr-sample', prune: null },
    ...(multi ? [{ id: 'lab', name: 'Lab server', host: 'lab.local', root: '/srv/fabr', prune: null }, { id: 'vm', name: 'VM', host: 'vm', root: '~/sample', prune: null }] : []),
  ];
  let settings: Settings = { poll_ms: 2000, remote_poll_ms: 30000, search_depth: 6, full_rescan_ms: 30000, max_nodes: 10000, log_path: '/tmp/fabr/fabr.log' };
  let fabSettings: FabricSettings = {
    default_folder: '~/', model: '', telegram_user_id: null, auto_mode: false,
    question_budget: 5, heartbeat_interval_s: 10, execute: 'full', capabilities: { tools: ['read_file', 'write_file', 'run_command'], checks: ['git', 'cargo'] }, setup: { completed_steps: [], completed: false },
    claude_bin: '', effort: '', max_budget_usd: 0, execute_timeout_s: 1800, replan_on_fail: true, propose_procedures: true,
  };
  const ready = typeof location !== 'undefined' && new URLSearchParams(location.search).has('ready');
  if (ready) fabSettings = { ...fabSettings, setup: { completed_steps: ['folder', 'claude'], completed: true } };
  let fabSettingsExist = ready;
  let know: NodeKnowledge = {
    rules: 'Ask before deleting files.\nKeep commits small.',
    knowledge: 'Tests run with npm test.\nThe API lives in src/server.',
    learned: [{ id: 'l1', text: 'Prefer pnpm over npm here.', applies_when: 'installing packages', scope: 'project', count: 3, source_tasks: [], last_confirmed: null },
      { id: 'l2', text: 'Ship to the home address, standard shipping.', applies_when: 'ordering parts online with a very long condition that wraps', scope: 'global', count: 5, source_tasks: [], last_confirmed: '2026-10-20T09:30:00Z', always: true },
    ],
  };
  let proposals: Proposal[] = [{ id: 'p1', applies_when: 'Adding an API route', steps: '- Add the handler in src/server/routes\n- Register it in src/server/index.ts\n- Run npm test', source_task: '2026-10-29-add-health-route', created_at: '2026-10-29T12:00:00Z' }];
  const takeProposal = (id: string) => {
    const p = proposals.find((x) => x.id === id);
    if (!p) throw new ApiError(404, `No proposal ${id}`);
    proposals = proposals.filter((x) => x.id !== id);
    return p;
  };
  const ccSessions: CcSession[] = [
    { id: 'mock-session-1', title: 'Fix the login redirect', modified_at: new Date(Date.now() - 3600_000).toISOString(), messages: 6, bytes: 4200 },
    { id: 'mock-session-2', title: '', modified_at: new Date(Date.now() - 3 * 86400_000).toISOString(), messages: 2, bytes: 900 },
    { id: 'mock-session-3', title: 'Order the filament', modified_at: new Date(Date.now() - 86400_000).toISOString(), messages: 4, bytes: 2100, task: { title: 'Order 2 kg of black PLA', role: 'plan' } },
  ];
  const ccExtra: Record<string, Transcript['entries']> = {};
  const ccTranscript = (id: string, limit: number): Transcript => {
    const all: Transcript['entries'] = [
      { role: 'user', text: 'The login page redirects to a blank screen.\nCan you look?' },
      { role: 'assistant', text: 'I will check the redirect handler first.' },
      { role: 'tool', text: 'src/auth/redirect.ts', tool: 'Read' },
      { role: 'tool', text: 'npm test', tool: 'Bash' },
      { role: 'assistant', text: 'The redirect used a relative path. Fixed it.' },
    ];
    all.push(...(ccExtra[id] ?? []));
    const entries = all.slice(-limit);
    return { id, title: ccSessions.find((s) => s.id === id)?.title ?? '', entries, truncated: entries.length < all.length };
  };
  const fabSecrets: FabricSecretsStatus = { telegram_bot_token: false };
  const fabric = new FixtureSource();
  const FAB_EVENTS = '/fixtures/home/.fabric-space/events.log';
  const qa = typeof location !== 'undefined' && new URLSearchParams(location.search).has('qa');
  const remote = remoteFixtures(today, qa);
  let rev = 1;
  const specs: Record<string, typeof SAMPLE> = { local: SAMPLE, lab: LAB, vm: qa ? VM_QA : VM };
  const PLAIN_DIRS = ['docs', 'notes', 'projects', 'node_modules'];
  const specFor = (src: Source): typeof SAMPLE =>
    specs[src.id] ?? [['', null, null], ...PLAIN_DIRS.map((d): (typeof SAMPLE)[number] => [d, null, null])];
  const PRUNED = new Set(['node_modules', '.git', 'target', 'dist', 'build', 'venv', '__pycache__']);
  const edits = new Map<string, Node>();
  const created = new Set<string>();
  const removed = new Set<string>();
  const trusted = new Set<string>();
  const passwords = new Map<string, string>();
  const keyInstalled = new Set<string>();
  const MOCK_PW = 'secret';
  const fail = (kind: TestResult['failure_kind'], message: string, detail: string): TestResult =>
    ({ ok: false, message, latency_ms: 0, node_count: 0, failure_kind: kind, detail });

  const wait = () => new Promise((r) => setTimeout(r, latency));
  const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));
  const key = (s: string, p: string) => `${s}:${p}`;
  const ver = () => (rev++).toString(16).padStart(40, '0');

  function sourceRoot(src: Source, showAll: boolean): Node {
    const root = buildSample(src.id, today, specFor(src).map(([path, meta, todos]): (typeof SAMPLE)[number] =>
      meta === null && created.has(key(src.id, path)) ? [path, { name: path.split('/').pop() || src.name }, []]
        : removed.has(key(src.id, path)) ? [path, null, null] : [path, meta, todos]), showAll);
    root.name = src.name;
    (function walk(n: Node) {
      const k = key(n.source, n.path);
      const e = edits.get(k);
      if (e) Object.assign(n, { ...e, children: n.children });
      else if (created.has(k)) Object.assign(n, { managed: true, version: ver(), meta: { name: n.name } });
      n.children.forEach(walk);
    })(root);
    return root;
  }

  let last: Tree | null = null;
  function tree(showAll: boolean): Tree {
    const roots = sources.map((s) => sourceRoot(s, showAll));
    const root: Node = {
      source: '', path: '', name: '/', managed: false, meta: {}, items: [], warnings: [],
      version: null, via: null, children: roots,
    };
    last = { root, sources: clone(sources), today, truncated: false, errors: [], pending: [] };
    return last;
  }

  function find(ref: NodeRef, all = false): Node {
    const look = (t: Tree): Node | null => {
      let hit: Node | null = null;
      (function walk(n: Node) {
        if (n.source === ref.source && n.path === ref.path) hit = n;
        n.children.forEach(walk);
      })(t.root);
      return hit;
    };
    const hit = (!all && last && look(last)) || look(tree(all));
    if (!hit) throw new ApiError(404, `no such node: ${ref.source}:${ref.path}`);
    return hit;
  }

  function write(ref: NodeRef, version: string, patch: Partial<Node>): Node {
    const n = find(ref);
    if (!n.managed) throw new ApiError(404, 'not a fabr node');
    if (n.version !== version) {
      throw new ApiError(409, 'node changed on disk', { ...clone(n), children: [] });
    }
    Object.assign(n, patch, { version: ver() });
    const saved = { ...clone(n), children: [] };
    edits.set(key(n.source, n.path), saved);
    return saved;
  }

  const api: Api = {
    async getTree(showAll) { await wait(); return clone(tree(showAll)); },
    async getStamp() { await wait(); return { stamp: String(rev) }; },
    async saveMeta(req) {
      await wait();
      const meta: Meta = { ...req.meta };
      return write(req, req.version, { meta });
    },
    async initNode(req) {
      await wait();
      const n = find(req);
      if (n.managed) throw new ApiError(409, 'already a node', { ...clone(n), children: [] });
      created.add(key(req.source, req.path));
      removed.delete(key(req.source, req.path));
      rev++;
      return { ...clone(n), managed: true, version: ver(), meta: { name: n.name } };
    },
    async removeNode(req) {
      await wait();
      if (!req.source) throw new ApiError(400, 'no source given');
      const n = find(req);
      if (!n.managed) throw new ApiError(404, 'directory is not a fabr node');
      if (n.version !== req.version) throw new ApiError(409, 'the node changed on disk since you loaded it', { ...clone(n), children: [] });
      const k = key(req.source, req.path);
      edits.delete(k);
      created.delete(k);
      removed.add(k);
      rev++;
      return { ...clone(n), managed: false, meta: {}, items: [], warnings: [], version: null, via: null, children: [] };
    },
    async listDirs(req): Promise<DirEntry[]> {
      await wait();
      const n = find(req, true);
      return n.children.map((c) => ({ name: c.name, managed: c.managed, pruned: false }))
        .concat(PLAIN_DIRS.filter((d) => PRUNED.has(d) && !n.children.some((c) => c.name === d) && !specs[n.source]).map((d) => ({ name: d, managed: false, pruned: true })))
        .sort((a, b) => a.name.localeCompare(b.name));
    },
    async initChildren(req) {
      await wait();
      const n = find(req, true);
      const names = new Set(n.children.map((c) => c.name));
      for (const name of req.names) {
        if (!names.has(name)) throw new ApiError(404, `no such directory: ${name}`);
      }
      created.add(key(req.source, req.path));
      for (const name of req.names) created.add(key(req.source, req.path ? `${req.path}/${name}` : name));
      rev++;
      return { ...clone(find(req, true)), children: [] };
    },
    async listSources() { await wait(); return clone(sources); },
    async saveSources(next) {
      await wait();
      for (const s of next) if (!/^[a-z0-9_-]+$/.test(s.id)) throw new ApiError(400, `source id ${JSON.stringify(s.id)} must match ^[a-z0-9_-]+$`);
      sources = clone(next);
      rev++;
      return clone(sources);
    },
    async testSource(s): Promise<TestResult> {
      if (!/^[a-z0-9_-]+$/.test(s.id)) throw new ApiError(400, `source id ${JSON.stringify(s.id)} must match ^[a-z0-9_-]+$`);
      await new Promise((r) => setTimeout(r, latency ? 400 : 0));
      const h = s.host ?? '';
      if (h.includes('bad')) return fail('connect_failed', `Could not connect: Could not resolve hostname ${h}: nodename nor servname provided, or not known`, `ssh: Could not resolve hostname ${h}: nodename nor servname provided, or not known`);
      if (h.includes('changed')) return fail('host_key_changed', `Host key changed: ${h} presents a different key than the one in your known_hosts`, `Host key for ${h} has changed and you have requested strict checking.`);
      if (h.includes('new') && !trusted.has(h)) return fail('host_key_unknown', `Unknown host key: ${h} is not in your known_hosts yet`, `No ED25519 host key is known for ${h} and you have requested strict checking.`);
      if (h.includes('pw') && !keyInstalled.has(h) && (s.password || passwords.get(s.id)) !== MOCK_PW) {
        return fail('auth_failed', 'Authentication failed: the host did not accept your key or password', `me@${h}: Permission denied (publickey,password).`);
      }
      return { ok: true, message: 'ok', latency_ms: s.host ? 84 : 3, node_count: specs[s.id] ? specs[s.id].length : 0, managed_root: !!specs[s.id] || created.has(key(s.id, '')) };
    },
    async hostKey(host) {
      await wait();
      if (host.includes('bad')) throw new ApiError(502, `Could not connect: no host keys from ${host}:22`);
      return [
        { type: 'ssh-ed25519', fingerprint: 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU', line: `${host} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl` },
        { type: 'ecdsa-sha2-nistp256', fingerprint: 'SHA256:p2QAMXNIC1TJYWeIOttrVc98/R1BUFWu3/LiyKgUfQM', line: `${host} ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY=` },
      ];
    },
    async trustHost(host, lines) {
      await wait();
      if (host.includes('changed')) throw new ApiError(409, `host_key_changed: your known_hosts already has a different key for ${host}; nothing was changed`);
      const added = trusted.has(host) ? 0 : lines.length;
      trusted.add(host);
      return { added };
    },
    async setSourcePassword(source, password) {
      if (!/^[a-z0-9_-]+$/.test(source)) throw new ApiError(400, `source id ${JSON.stringify(source)} must match ^[a-z0-9_-]+$`);
      await wait();
      if (password) passwords.set(source, password);
      else passwords.delete(source);
      return { stored: !!password };
    },
    async hasSourcePassword(source) { await wait(); return { stored: passwords.has(source) }; },
    async fabricList() {
      const f = (r: { path: string }) => ({ path: r.path, mtime: 0 });
      return { records: (await fabric.listRecords()).map(f), registry: (await fabric.listRegistry()).map(f), events: { path: FAB_EVENTS, mtime: 0 } };
    },
    async fabricRead(path) {
      if (path === FAB_EVENTS) return { text: (await fabric.readEvents()).join('\n'), mtime: 0 };
      const reg = (await fabric.listRegistry()).find((r) => r.path === path);
      return { text: reg ? reg.text : await fabric.readRecord(path), mtime: 0 };
    },
    async fabricWrite(req) {
      try { await fabric.writeRecord(req.path, req.text, req.expected_updated_at); } catch (e) {
        const m = (e as Error).message;
        throw new ApiError(m.startsWith('state may not change') ? 403 : 409, m);
      }
      return { mtime: 0 };
    },
    async fabricCreate(req) {
      try { return await fabric.createRecord(req.folder, req); } catch (e) { throw new ApiError(400, (e as Error).message); }
    },
    async importTodos() { throw new ApiError(501, 'import_todos is not available in the browser'); },
    async fabricStamp() { return { stamp: await fabric.stamp() }; },
    async fabricHome() { return '/home/mock'; },
    async fabricRemoteList(source) { return remote.list(source); },
    async fabricRemoteStamp(source) { remote.need(source); return { stamp: remote.stamp() }; },
    async fabricRemoteWrite(req) { return remote.write(req.source, req.path, req.text, req.expected_updated_at); },
    async fabricRemoteCreate(req) { return remote.create(req.source, req.folder, req); },
    async fabricSettingsRead() { await wait(); return { exists: fabSettingsExist, settings: clone(fabSettings) }; },
    async fabricSettingsWrite(s) { await wait(); fabSettings = clone(s); fabSettingsExist = true; return { exists: true, settings: clone(fabSettings) }; },
    async fabricSecretsWrite(req) {
      await wait();
      if (req.telegram_bot_token) fabSecrets.telegram_bot_token = true;
      return { ...fabSecrets };
    },
    async fabricSecretsStatus() { await wait(); return { ...fabSecrets }; },
    async fabricOpenSession() { throw new ApiError(501, 'Opening a terminal works in the desktop app only'); },
    async fabricOpenClaude() { throw new ApiError(501, 'Opening a terminal works in the desktop app only'); },
    async fabricCcSessions() { await wait(); return clone(ccSessions); },
    async fabricCcTranscript(_f, id, limit = 300) {
      await wait();
      if (!ccSessions.some((s) => s.id === id)) throw new ApiError(404, 'No such session');
      return ccTranscript(id, limit);
    },
    async fabricCcTrack(_f, id) {
      await wait();
      const s = ccSessions.find((x) => x.id === id);
      if (!s) throw new ApiError(404, 'No such session');
      if (s.task) throw new ApiError(409, 'This session is already tracked by a task.');
      s.task = { title: s.title || 'Untitled session', role: 'plan' };
      return { path: `/fixtures/home/.fabric-space/tasks/${id}.md` };
    },
    async fabricCcSend({ id, message }) {
      await wait();
      if (!message.trim()) throw new ApiError(400, 'Message is empty');
      if (!ccSessions.some((s) => s.id === id)) throw new ApiError(404, 'No such session');
      const reply = 'I looked at the files. The redirect handler is in src/auth/redirect.ts and the fix is small.';
      (ccExtra[id] ??= []).push({ role: 'user', text: message }, { role: 'assistant', text: reply });
      return { reply, cost_usd: 0.11 };
    },
    async fabricKnowledgeRead() { await wait(); return clone(know); },
    async fabricKnowledgeWrite(_f, kind: KnowledgeKind, text) { await wait(); know = { ...know, [kind]: text }; return clone(know); },
    async fabricLearnedForget(_f, id) { await wait(); know = { ...know, learned: know.learned.filter((l) => l.id !== id) }; return clone(know); },
    async fabricLearnedAlways(_f, id, always) {
      await wait();
      if (!know.learned.some((l) => l.id === id)) throw new ApiError(404, `No learned record ${id}`);
      know = { ...know, learned: know.learned.map((l) => (l.id === id ? { ...l, always } : l)) };
      return clone(know);
    },
    async fabricProposalsRead() { await wait(); return clone(proposals); },
    async fabricProposalAccept(_f, id) {
      await wait();
      const p = takeProposal(id);
      const section = `### Procedure: ${p.applies_when}\n${p.steps}`;
      know = { ...know, knowledge: know.knowledge.trim() ? `${know.knowledge.trimEnd()}\n\n${section}` : section };
      return clone(proposals);
    },
    async fabricProposalDismiss(_f, id) { await wait(); takeProposal(id); return clone(proposals); },
    async fabricAgentStatus() { return { running: true, reason: null }; },
    async fabricAgentStart() { return { running: true, reason: null }; },
    async fabricCheck(req) {
      await wait();
      if (req.kind === 'claude') return { ok: true, version: '2.1.287 (Claude Code)', path: req.path || '/usr/local/bin/claude', logged_in: true, auth: 'claude.ai, max' };
      if (req.kind === 'telegram_token') return { ok: true, bot_name: 'fabr_mock_bot' };
      return { ok: true };
    },
    async fabricTelegramLink() { await wait(); return { ok: true, user_id: 123456789 }; },
    async installKey(req) {
      await wait();
      const host = req.host ?? sources.find((s) => s.id === req.source)?.host ?? '';
      if (!host) throw new ApiError(400, `source "${req.source}" has no ssh host`);
      if (host.includes('pw') && (req.password || passwords.get(req.source)) !== MOCK_PW && !keyInstalled.has(host)) {
        throw new ApiError(502, 'Authentication failed: the host did not accept your key or password');
      }
      keyInstalled.add(host);
      return { installed: true, generated_key: false, message: `Key installed: ${host} now accepts ~/.ssh/id_ed25519 without a password.` };
    },
    async getSettings() { await wait(); return clone(settings); },
    async saveSettings(next) { await wait(); settings = { ...settings, ...clone(next) }; return clone(settings); },
    async revealPath() {
      await wait();
      throw new ApiError(501, 'reveal_path is only available in the desktop app');
    },
    async openUrl() {},
    async openPath(req) {
      await wait();
      const src = sources.find((s) => s.id === req.source);
      if (src?.host) throw new ApiError(400, 'cannot open a remote path');
      throw new ApiError(501, 'open_path is not available in the browser');
    },
  };
  return api;
}

const RICH_PLAN = {
  tools: ['Write', 'Edit(//etc/hosts)', 'Bash(python3 -m unittest:*)', 'WebFetch'],
  checks: [{ cmd: 'python3', args: ['-m', 'unittest', 'discover'] }, { cmd: 'git', args: ['diff', '--stat'] }],
};

function richer(r: TaskRecord): TaskRecord {
  return { ...r, sessions: { ...r.sessions, plan: 'mock-session-1' }, plan_meta: { ...r.plan_meta!, tools: RICH_PLAN.tools, checks: RICH_PLAN.checks } };
}

function replanned(r: TaskRecord): TaskRecord {
  const log = `${r.sections.log}\n- 2026-10-24T19:02:30Z evaluate: replanning after failed check (a2)`;
  return {
    ...richer(r),
    verdict: {
      result: 'fail', at: '2026-10-24T19:02:25Z',
      items: [
        { id: 'a1', pass: true, evidence: 'audit.log shows only granted fields were filled' },
        { id: 'a2', pass: false, evidence: 'cart shows 1 x black PLA 1 kg, 1 kg short; the shop allows 1 spool per order' },
      ],
    },
    sections: { ...r.sections, log, logEntries: [...r.sections.logEntries, { ts: '2026-10-24T19:02:30Z', step: 'evaluate', message: 'replanning after failed check (a2)' }] },
  };
}

function remoteFixtures(today: string, qa = false) {
  const fx = new FixtureSource();
  type Pick = [node: string, fixture: string, title: string, patch?: (r: TaskRecord) => TaskRecord];
  const PICK: Pick[] = [
    ['', '01-reminder', 'Check printer firmware'],
    ['', '05-running', 'Order filament'],
    ['app', '02-open-question', 'Order filament spools'],
    ['app', '04-awaiting-approval', 'Approve the release plan', qa ? richer : undefined],
    ...(qa ? [
      ['app', '03-choice-question', 'Choose the shipping address'] as Pick,
      ['app', '04-awaiting-approval', 'Re-plan the release', replanned] as Pick,
    ] : []),
  ];
  const root = '~/sample';
  const dirOf = (node: string) => `${root}${node ? `/${node}` : ''}/.fabric-space/tasks`;
  const files = new Map<string, string>();
  let registry: { name: string; text: string }[] = [];
  let writes = 0;
  const ready = (async () => {
    const all = await fx.listRecords();
    const regs = await fx.listRegistry();
    for (const [node, fixture, title, patch] of PICK) {
      const src = all.find((r) => r.path.includes(`/${fixture}/`));
      if (!src) continue;
      const id = `${today}-${taskSlug(title)}`;
      const path = `${dirOf(node)}/${id}.md`;
      const rec = { ...parseRecord(src.text), id, title };
      files.set(path, serializeRecord(patch ? patch(rec) : rec));
      if (fixture === '05-running') {
        const reg = regs.find((r) => r.path.includes('/05-running/'));
        if (reg) registry = [{ name: `${id}--vm.yaml`, text: reg.text.replace(/^task_path:.*$/m, `task_path: ${path}`).replace(/^task_id:.*$/m, `task_id: ${id}`).replace(/^folder:.*$/m, `folder: ${root}`) }];
      }
    }
  })();
  const need = (s: Source) => { if (!s.host) throw new ApiError(400, `source ${s.id} is not remote`); };
  return {
    need,
    stamp: () => `${writes}:${Math.floor(Date.now() / 5000)}`,
    async list(s: Source) {
      need(s);
      await ready;
      const at = new Date().toISOString();
      return {
        records: [...files.keys()].sort().map((path) => ({ path, text: files.get(path)! })),
        registry: registry.map((r) => ({ ...r, text: r.text.replace(/^heartbeat_at:.*$/m, `heartbeat_at: ${at}`) })),
        events: '',
        backend: `heartbeat_at: ${at}\n`,
        stamp: `${writes}`,
      };
    },
    async write(s: Source, path: string, text: string, expected: string) {
      need(s);
      await ready;
      const cur = files.get(path);
      if (cur === undefined) throw new ApiError(404, `no record at ${path}`);
      if (updatedAtOf(cur) !== expected) throw new ApiError(409, `${path} changed: updated_at is "${updatedAtOf(cur)}", expected "${expected}"`);
      try { files.set(path, guardState(cur, text)); } catch (e) { throw new ApiError(403, (e as Error).message); }
      writes += 1;
      return { mtime: 0 };
    },
    async create(s: Source, folder: string, f: { title: string; due?: string | null; pri?: number; context?: string; spec?: string; success?: string }) {
      need(s);
      await ready;
      if (!f.title.trim()) throw new ApiError(400, 'a task needs a title');
      const base = `${today}-${taskSlug(f.title)}`;
      let id = base;
      for (let n = 2; files.has(`${folder.replace(/\/+$/, '')}/.fabric-space/tasks/${id}.md`); n++) id = `${base}-${n}`;
      const path = `${folder.replace(/\/+$/, '')}/.fabric-space/tasks/${id}.md`;
      files.set(path, serializeRecord(newRecord(id, f)));
      writes += 1;
      return { path };
    },
  };
}
