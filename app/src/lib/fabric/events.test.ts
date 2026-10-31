import { beforeEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import NoticesBadge from './NoticesBadge.svelte';
import {
  loadReadPosition, markAllRead, parseEventLine, parseEvents, reconcileReadPosition, saveReadPosition, taskIdOf, unreadCount,
} from './events';

const logs = import.meta.glob('../../../../fixtures/**/home/.fabric-space/events.log', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const lineOf = (name: string) => {
  const key = Object.keys(logs).find((k) => k.includes(name))!;
  return logs[key].split('\n').filter((l) => l.trim() !== '');
};

describe('parseEventLine', () => {
  it.each([
    ['06-waiting', 'waiting', '2026-10-24-order-filament'],
    ['07-succeeded', 'done', '2026-10-24-order-filament'],
    ['08-failed', 'failed', '2026-10-24-order-filament'],
    ['10-registry', 'stuck', '2026-10-24-rename-scans'],
  ])('fixture %s', (fx, kind, id) => {
    const lines = lineOf(fx);
    expect(lines.length).toBeGreaterThan(0);
    const e = parseEventLine(lines[0])!;
    expect(e.kind).toBe(kind);
    expect(taskIdOf(e.taskPath)).toBe(id);
    expect(e.taskPath.endsWith('.md')).toBe(true);
    expect(e.text.length).toBeGreaterThan(0);
  });
  it('parses a promotion notice', () => {
    const e = parseEventLine('2026-10-31T08:00:00Z learned /f/.fabric-space/.agents/learned.yaml promoted l3 to global (confirmed in pets and team-notes)')!;
    expect([e.kind, e.text]).toEqual(['learned', 'promoted l3 to global (confirmed in pets and team-notes)']);
  });
  it('returns null for malformed lines', () => {
    expect(parseEventLine('garbage')).toBeNull();
    expect(parseEventLine('2026-10-24T18:59:31Z exploded /a/b.md text')).toBeNull();
    expect(parseEventLine('2026-10-24T18:59:31Z done')).toBeNull();
    expect(parseEventLine('')).toBeNull();
  });
});

describe('parseEvents and unreadCount', () => {
  const lines = [
    '2026-10-24T18:00:00Z waiting /a/t/one.md q',
    'junk',
    '2026-10-24T19:00:00Z done    /a/t/two.md ok',
    '2026-10-24T18:30:00Z stuck   /a/t/three.md dead',
  ];
  const events = parseEvents(lines);
  it('orders newest first and drops malformed lines', () => {
    expect(events.map((e) => taskIdOf(e.taskPath))).toEqual(['two', 'three', 'one']);
  });
  it('counts unread at positions 0, mid and full', () => {
    expect(unreadCount(events, 0)).toBe(3);
    expect(unreadCount(events, 1)).toBe(2);
    expect(unreadCount(events, markAllRead(events))).toBe(0);
    expect(unreadCount(events, 99)).toBe(0);
  });
});

describe('read position store', () => {
  beforeEach(() => {
    const m = new Map<string, string>();
    (globalThis as any).localStorage = {
      getItem: (k: string) => m.get(k) ?? null,
      setItem: (k: string, v: string) => void m.set(k, v),
    };
  });
  it('survives a write and read, per path', () => {
    saveReadPosition('/h/events.log', 4);
    expect(loadReadPosition('/h/events.log')).toBe(4);
    expect(loadReadPosition('/other/events.log')).toBe(0);
  });
  it('does not throw without localStorage', () => {
    delete (globalThis as any).localStorage;
    expect(() => saveReadPosition('/h/events.log', 2)).not.toThrow();
    expect(loadReadPosition('/h/events.log')).toBe(0);
  });
});

describe('truncated log', () => {
  beforeEach(() => {
    const m = new Map<string, string>();
    (globalThis as any).localStorage = {
      getItem: (k: string) => m.get(k) ?? null,
      setItem: (k: string, v: string) => void m.set(k, v),
    };
  });
  const mk = (n: number) => parseEvents(Array.from({ length: n }, (_, i) => `2026-10-24T18:0${i}:00Z done /a/t${i}.md ok`));
  it('clamps a position beyond the count and saves it', () => {
    saveReadPosition('/h/events.log', 9);
    const p = reconcileReadPosition('/h/events.log', loadReadPosition('/h/events.log'), 3);
    expect(p).toBe(3);
    expect(loadReadPosition('/h/events.log')).toBe(3);
    expect(unreadCount(mk(3), 9)).toBe(0);
    expect(unreadCount(mk(4), p)).toBe(1);
  });
  it('leaves a valid position alone', () => {
    expect(reconcileReadPosition('/h/events.log', 2, 3)).toBe(2);
  });
});

describe('NoticesBadge', () => {
  const events = parseEvents(['2026-10-24T18:00:00Z waiting /a/one.md q', '2026-10-24T19:00:00Z done /a/two.md ok']);
  it('shows the count of unread', () => {
    const { body } = render(NoticesBadge, { props: { events, readPosition: 0 } });
    expect(body).toContain('>2<');
  });
  it('is hidden at zero', () => {
    const { body } = render(NoticesBadge, { props: { events, readPosition: 2 } });
    expect(body).not.toContain('nbadge');
  });
});
