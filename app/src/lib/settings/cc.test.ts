import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import { createMockApi } from '../../mock/api';
import { ago, newestFirst, nodeFolder, sessionTitle } from '../fabric/cc';
import TranscriptView from '../fabric/TranscriptView.svelte';
import ApprovalPanel from '../fabric/ApprovalPanel.svelte';
import SessionsBlock from '../fabric/SessionsBlock.svelte';
import TranscriptSheet from '../fabric/TranscriptSheet.svelte';
import KnowledgeGroup from './KnowledgeGroup.svelte';
import QuestionPanel from '../fabric/QuestionPanel.svelte';

const api = createMockApi({ latency: 0 });
const ID2 = '3f2b8c1e-4d5a-4e6f-9a7b-0c1d2e3f4a5b';

describe('mock claude code api', () => {
  it('lists sessions and reads a transcript with a limit', async () => {
    const list = await api.fabricCcSessions('/x');
    expect(list.length).toBe(2);
    const t = await api.fabricCcTranscript('/x', list[0].id, 2);
    expect(t.entries.length).toBe(2);
    expect(t.truncated).toBe(true);
    await expect(api.fabricCcTranscript('/x', 'nope')).rejects.toThrow();
  });
  it('sends a message and the transcript shows it with the reply', async () => {
    const r = await api.fabricCcSend({ folder: '/x', id: 'mock-session-1', message: 'What changed?' });
    expect(r.reply.length).toBeGreaterThan(0);
    expect(r.cost_usd).toBeGreaterThan(0);
    const t = await api.fabricCcTranscript('/x', 'mock-session-1');
    expect(t.entries.slice(-2).map((e) => [e.role, e.text])).toEqual([['user', 'What changed?'], ['assistant', r.reply]]);
    await expect(api.fabricCcSend({ folder: '/x', id: 'nope', message: 'hi' })).rejects.toThrow();
    await expect(api.fabricCcSend({ folder: '/x', id: 'mock-session-1', message: ' ' })).rejects.toThrow();
  });
  it('writes knowledge and forgets learned records', async () => {
    const k = await api.fabricKnowledgeWrite('/x', 'rules', 'Be brief.');
    expect(k.rules).toBe('Be brief.');
    const id = k.learned[0].id;
    expect((await api.fabricLearnedForget('/x', id)).learned.find((l) => l.id === id)).toBeUndefined();
  });
  it('turns always on and off for a learned record', async () => {
    const api = createMockApi({ latency: 0 });
    const id = (await api.fabricKnowledgeRead('/x')).learned[0].id;
    expect((await api.fabricLearnedAlways('/x', id, true)).learned[0].always).toBe(true);
    expect((await api.fabricLearnedAlways('/x', id, false)).learned[0].always).toBe(false);
    await expect(api.fabricLearnedAlways('/x', 'nope', true)).rejects.toThrow();
  });
});

describe('helpers', () => {
  it('formats time, titles, order and folders', () => {
    const now = new Date('2026-01-10T12:00:00Z');
    expect(ago('2026-01-10T11:00:00Z', now)).toBe('1 h ago');
    expect(ago('2026-01-07T12:00:00Z', now)).toBe('3 d ago');
    expect(sessionTitle({ title: ' ' })).toBe('Untitled');
    expect(newestFirst([{ modified_at: 'a' }, { modified_at: 'b' }] as never).map((s) => s.modified_at)).toEqual(['b', 'a']);
    expect(nodeFolder('~/fabr', 'a/b', '/home/me')).toBe('/home/me/fabr/a/b');
    expect(nodeFolder('/srv/x/', '', '/home/me')).toBe('/srv/x');
  });
});

describe('render', () => {
  it('transcript viewer shows messages, tool lines and the truncation note', () => {
    const transcript = {
      id: 'a', title: '', truncated: true,
      entries: [
        { role: 'user' as const, text: 'Hi\nthere' }, { role: 'assistant' as const, text: 'Hello' },
        { role: 'tool' as const, text: 'src/a.ts', tool: 'Read' }, { role: 'tool' as const, text: 'boom', tool: 'error' },
      ],
    };
    const { body } = render(TranscriptView, { props: { transcript } });
    for (const t of ['Showing the last 4 entries', 'Hi\nthere', 'Hello', 'Read  src/a.ts', 'tool--bad']) expect(body).toContain(t);
  });
  it('knowledge group shows both fields and learned records with Forget', () => {
    const k = { rules: 'R1', knowledge: 'K1', learned: [{ id: 'l1', text: 'Use pnpm', applies_when: 'installing', scope: 'project', count: 3, source_tasks: [] }] };
    const props = { k, onsave: () => {}, onforget: () => {}, onalways: () => {} };
    const { body } = render(KnowledgeGroup, { props });
    for (const t of ['Rules', 'Knowledge', 'How the agent must behave in this folder', 'Facts about this folder', 'R1', 'K1', 'Use pnpm', 'When installing', 'x3', 'Forget', 'data-testid="always-l1"']) expect(body).toContain(t);
    expect(body).not.toContain('Stop always');
    const on = render(KnowledgeGroup, { props: { ...props, k: { ...k, learned: [{ ...k.learned[0], always: true }] } } }).body;
    for (const t of ['Always · When installing', 'Stop always']) expect(on).toContain(t);
  });
  it('a choice question shows the prior and the three answers', () => {
    const question = { id: 'q2', kind: 'choice' as const, text: 'Ship home again?', options: ['once', 'always', 'no'], prior: { text: 'Ship to home', source_task: '/p/.fabric-space/tasks/2026-10-01-order.md', last_confirmed: '2026-10-01T00:00:00Z' }, fields: null, asked_at: '2026-10-31T10:00:00Z', answer: null };
    const { body } = render(QuestionPanel, { props: { question, onsubmit: async () => {} } });
    for (const t of ['Ship home again?', 'Last time: Ship to home', 'From 2026-10-01-order, confirmed 2026-10-01', 'Once', 'Always', 'No, something else']) expect(body).toContain(t);
  });
  const ID = '3f2b8c1e-4d5a-4e6f-9a7b-0c1d2e3f4a5b';
  const approval = { status: 'pending' as const, by: null, at: null, note: null, digest: null };
  const grants = { requested: [], granted: [] };
  it('approval panel offers to discuss the plan with the hint and redo', () => {
    const props = { approval, grants, planMeta: null, plan: 'P', assertions: [], onsubmit: async () => {}, folder: '/x', planSession: ID, onredo: () => {} };
    const { body } = render(ApprovalPanel, { props });
    for (const t of ['Discuss plan in Claude Code', 'When you are done, press Redo plan and the planner picks up the conversation.', 'Redo plan']) expect(body).toContain(t);
    expect(render(ApprovalPanel, { props: { ...props, planSession: null } }).body).not.toContain('Discuss plan');
  });
  const planMeta = { reversibility: 'reversible' as const, tools: ['Write', 'Bash(npm test:*)', 'WebFetch'], requires: { tools: [], checks: [] }, assumptions: [] as { rule_id: string; text: string }[], checks: [{ cmd: 'test', args: ['-f', 'greet.md'] }] };
  it('approval panel lists assumed always-rules only when there are some', () => {
    const props = { approval, grants, planMeta, plan: 'P', assertions: [], onsubmit: async () => {} };
    expect(render(ApprovalPanel, { props }).body).not.toContain('Assumed');
    const { body } = render(ApprovalPanel, { props: { ...props, planMeta: { ...planMeta, assumptions: [{ rule_id: 'l1', text: 'Ship to home' }] } } });
    for (const t of ['Assumed', 'Ship to home', 'Applied because you chose Always.']) expect(body).toContain(t);
  });
  it('approval panel shows what Claude Code may use above Approve', () => {
    const props = { approval, grants, planMeta, plan: 'P', assertions: [], onsubmit: async () => {} };
    const { body } = render(ApprovalPanel, { props });
    for (const t of ['Claude Code may use', 'Write', 'test -f greet.md']) expect(body).toContain(t);
    expect(body).toMatch(/chip muted[^"]*">Read</);
    expect(body).toMatch(/hot">Bash\(npm test:\*\)<!--\[0--><small[^>]*>runs commands/);
    expect(body).toMatch(/hot">WebFetch<!--\[0--><small[^>]*>uses the network/);
    expect(body.indexOf('Claude Code may use')).toBeLessThan(body.indexOf('Approve'));
    expect(body).not.toContain('Irreversible');
    expect(render(ApprovalPanel, { props: { ...props, planMeta: { ...planMeta, reversibility: 'irreversible' as const } } }).body).toContain('Irreversible: once this runs it cannot be undone.');
  });
  it('sessions block puts the discuss action on the planner row only', () => {
    const one = render(SessionsBlock, { props: { sessions: { plan: ID, execute: null, evaluate: null }, folder: '/x' } }).body;
    expect(one).toContain('session-discuss');
    const two = render(SessionsBlock, { props: { sessions: { plan: null, execute: ID, evaluate: null }, folder: '/x' } }).body;
    expect(two).not.toContain('session-discuss');
  });
});

describe('composer', () => {
  it('transcript sheet shows the composer, the read-only note and a disabled Send', () => {
    const { body } = render(TranscriptSheet, { props: { folder: '/x', id: ID2, onclose: () => {} } });
    for (const t of ['Ask this session…', 'Read-only: Claude Code can look at files here but not change them.', 'data-testid="send"']) expect(body).toContain(t);
    expect(body).toMatch(/<button[^>]*data-testid="send"[^>]*disabled|<button[^>]*disabled[^>]*data-testid="send"/);
    expect(body).not.toContain('Redo plan');
  });
  it('discuss mode keeps the redo hint in the sheet and both panels offer Discuss here', () => {
    const { body } = render(TranscriptSheet, { props: { folder: '/x', id: ID2, discuss: true, onclose: () => {} } });
    expect(body).toContain('press Redo plan');
    const a = { approval: { status: 'pending' as const, by: null, at: null, note: null, digest: null }, grants: { requested: [], granted: [] }, planMeta: null, plan: 'P', assertions: [], onsubmit: async () => {}, folder: '/x', planSession: ID2 };
    expect(render(ApprovalPanel, { props: a }).body).toContain('Discuss here');
    expect(render(SessionsBlock, { props: { sessions: { plan: ID2, execute: null, evaluate: null }, folder: '/x' } }).body).toContain('session-discuss-here');
  });
});

describe('claude limits', () => {
  it('round trips through the mock settings', async () => {
    const r = await api.fabricSettingsRead();
    expect([r.settings.max_budget_usd, r.settings.execute_timeout_s]).toEqual([0, 1800]);
    const w = await api.fabricSettingsWrite({ ...r.settings, max_budget_usd: 2.5, execute_timeout_s: 600 });
    expect([w.settings.max_budget_usd, w.settings.execute_timeout_s]).toEqual([2.5, 600]);
  });
  it('mock claude check reports sign-in', async () => {
    expect(await api.fabricCheck({ kind: 'claude' })).toMatchObject({ ok: true, logged_in: true, auth: 'claude.ai, max' });
  });
});
