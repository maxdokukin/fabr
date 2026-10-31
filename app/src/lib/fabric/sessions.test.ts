import { describe, expect, it } from 'vitest';
import { sessionCommand, sessionList, shellQuote } from './sessions';

const ID = '3f2b8c1e-4d5a-4e6f-9a7b-0c1d2e3f4a5b';

describe('sessions', () => {
  it('lists only the sessions that exist, in pipeline order', () => {
    expect(sessionList({ plan: ID, execute: null, evaluate: 'b' })).toEqual([
      { key: 'plan', label: 'Planner', id: ID },
      { key: 'evaluate', label: 'Validator', id: 'b' },
    ]);
    expect(sessionList({ plan: null, execute: null, evaluate: null })).toEqual([]);
  });

  it('builds the resume command in the node folder', () => {
    expect(sessionCommand('/home/me/work', ID)).toBe(`cd /home/me/work && claude --resume ${ID}`);
  });

  it('quotes folders with spaces and quotes', () => {
    expect(shellQuote("/home/me/it's here")).toBe(`'/home/me/it'\\''s here'`);
    expect(sessionCommand('/Users/me/My Notes', ID)).toBe(`cd '/Users/me/My Notes' && claude --resume ${ID}`);
  });
});
