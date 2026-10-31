export type Seg = { code: boolean; bold?: boolean; text: string };
export type Block = { kind: 'h' | 'li' | 'p' | 'gap'; segs: Seg[] };

export function segs(line: string): Seg[] {
  const out: Seg[] = [];
  line.split('`').forEach((part, i) => {
    if (i % 2 === 1) { if (part) out.push({ code: true, text: part }); return; }
    part.split('**').forEach((t, j) => { if (t) out.push({ code: false, bold: j % 2 === 1, text: t }); });
  });
  return out;
}

export function proseBlocks(text: string): Block[] {
  const out: Block[] = [];
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd();
    const head = /^#{1,6}\s+(.*)$/.exec(line);
    const item = /^\s*(?:[-*]|\d+\.)\s+(.*)$/.exec(line);
    if (!line.trim()) { if (out.length && out[out.length - 1].kind !== 'gap') out.push({ kind: 'gap', segs: [] }); }
    else if (head) out.push({ kind: 'h', segs: segs(head[1]) });
    else if (item) out.push({ kind: 'li', segs: segs(line.trim()) });
    else out.push({ kind: 'p', segs: segs(line) });
  }
  while (out.length && out[out.length - 1].kind === 'gap') out.pop();
  return out;
}
