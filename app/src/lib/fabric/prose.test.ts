import { describe, expect, it } from 'vitest';
import { proseBlocks } from './prose';

describe('proseBlocks', () => {
  it('splits headings, list items, code and gaps', () => {
    const b = proseBlocks('### Steps\n1. Read `plants.md`\n- one\n\ntext\n');
    expect(b.map((x) => x.kind)).toEqual(['h', 'li', 'li', 'gap', 'p']);
    expect(b[0].segs[0].text).toBe('Steps');
    expect(b[1].segs.map((s) => s.code)).toEqual([false, true]);
  });
});

describe('bold', () => {
  it('marks ** spans', async () => {
    const { segs } = await import('./prose');
    expect(segs('a **b** `c`')).toEqual([{ code: false, bold: false, text: 'a ' }, { code: false, bold: true, text: 'b' }, { code: false, bold: false, text: ' ' }, { code: true, text: 'c' }]);
  });
});
