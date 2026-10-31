import { describe, expect, it } from 'vitest';
import { parseYaml } from './yaml';

describe('single-quoted scalars', () => {
  it('unescapes doubled quotes inside a value', () => {
    const y = parseYaml("a:\n- id: a2\n  text: 'Each plant''s line is ok: yes'\n") as { a: { text: string }[] };
    expect(y.a[0].text).toBe("Each plant's line is ok: yes");
  });
});
