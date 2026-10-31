export type EventKind = 'waiting' | 'done' | 'failed' | 'stuck' | 'learned';

export interface FabricEvent {
  at: string;
  kind: EventKind;
  taskPath: string;
  text: string;
  source?: string | null;
}

const KINDS: readonly string[] = ['waiting', 'done', 'failed', 'stuck', 'learned'];
const LINE = /^(\d{4}-\d\d-\d\dT[\d:.]+Z)\s+(\S+)\s+(\S+)(?:\s+(.*))?$/;

export function parseEventLine(line: string): FabricEvent | null {
  const m = LINE.exec(line.trim());
  if (!m || !KINDS.includes(m[2])) return null;
  return { at: m[1], kind: m[2] as EventKind, taskPath: m[3], text: (m[4] ?? '').trim() };
}

// Newest first; malformed lines are dropped. Ties on time keep the later log line first.
export function parseEvents(lines: string[]): FabricEvent[] {
  const out: { e: FabricEvent; i: number }[] = [];
  lines.forEach((l, i) => {
    const e = parseEventLine(l);
    if (e) out.push({ e, i });
  });
  out.sort((a, b) => (a.e.at < b.e.at ? 1 : a.e.at > b.e.at ? -1 : b.i - a.i));
  return out.map((x) => x.e);
}

// events is newest first; readPosition is how many events were seen when last marked read.
export function unreadCount(events: FabricEvent[], readPosition: number): number {
  return events.length - clampPosition(readPosition, events.length);
}

export function unreadEvents(events: FabricEvent[], readPosition: number): FabricEvent[] {
  return events.slice(0, unreadCount(events, readPosition));
}

export function markAllRead(events: FabricEvent[]): number {
  return events.length;
}

export function isUrgent(kind: EventKind): boolean {
  return kind === 'waiting' || kind === 'stuck';
}

export function taskIdOf(taskPath: string): string {
  const name = taskPath.split('/').pop() ?? taskPath;
  return name.replace(/\.md$/, '');
}

const keyFor = (eventsPath: string) => `fabric.events.read:${eventsPath}`;

export function clampPosition(position: number, count: number): number {
  return Math.min(Math.max(0, position), count);
}

export function reconcileReadPosition(eventsPath: string, position: number, count: number): number {
  const p = clampPosition(position, count);
  if (p !== position) saveReadPosition(eventsPath, p);
  return p;
}

export function loadReadPosition(eventsPath: string): number {
  try {
    const n = Number.parseInt(globalThis.localStorage.getItem(keyFor(eventsPath)) ?? '', 10);
    return Number.isFinite(n) && n > 0 ? n : 0;
  } catch {
    return 0;
  }
}

export function saveReadPosition(eventsPath: string, position: number): void {
  try {
    globalThis.localStorage.setItem(keyFor(eventsPath), String(Math.max(0, Math.floor(position))));
  } catch {
    /* storage unavailable */
  }
}
