import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import { createMockApi } from '../../mock/api';
import { ago, newestFirst, nodeFolder, sessionTitle } from '../fabric/cc';
import TranscriptView from '../fabric/TranscriptView.svelte';
import KnowledgeGroup from './KnowledgeGroup.svelte';

const api = createMockApi({ latency: 0 });

describe('mock claude code api', () => {
  it('lists sessions and reads a transcript with a limit', async () => {
    const list = await api.fabricCcSessions('/x');
    expect(list.length).toBe(2);
    const t = await api.fabricCcTranscript('/x', list[0].id, 2);
    expect(t.entries.length).toBe(2);
    expect(t.truncated).toBe(true);
    await expect(api.fabricCcTranscript('/x', 'nope')).rejects.toThrow();
  });
  it('writes knowledge and forgets learned records', async () => {
    const k = await api.fabricKnowledgeWrite('/x', 'rules', 'Be brief.');
    expect(k.rules).toBe('Be brief.');
    const id = k.learned[0].id;
    expect((await api.fabricLearnedForget('/x', id)).learned.find((l) => l.id === id)).toBeUndefined();
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
    const { body } = render(KnowledgeGroup, { props: { k, onsave: () => {}, onforget: () => {} } });
    for (const t of ['Rules', 'Knowledge', 'How the agent must behave in this folder', 'Facts about this folder', 'R1', 'K1', 'Use pnpm', 'When installing', 'x3', 'Forget']) expect(body).toContain(t);
  });
});
