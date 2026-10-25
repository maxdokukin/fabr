import { describe, expect, it } from 'vitest';
import { parseFrontMatter, parseRecord } from './parse';
import { serializeRecord } from './serialize';
import { FixtureSource } from './source';
import { FIELDS } from './types';

const records = await new FixtureSource().listRecords();

describe('serializeRecord', () => {
  for (const { path, text } of records) {
    const name = path.split('fixtures/')[1];
    it(`round-trips ${name}`, () => {
      const out = serializeRecord(parseRecord(text));
      expect(parseRecord(out)).toEqual({ ...parseRecord(text), schema: 6 });
      expect(Object.keys(parseFrontMatter(out).data)).toEqual([...FIELDS]);
    });
  }

  it('quotes strings that would read back as something else', () => {
    const r = parseRecord(records[0].text);
    const tricky = ['no', 'true', '123', '', 'a: b', 'x # y', ' lead', 'say "hi"', 'back\\slash', 'two\nlines', '[a]', '- dash'];
    for (const t of tricky) {
      const out = parseRecord(serializeRecord({ ...r, title: t }));
      expect(out.title).toBe(t);
    }
  });

  it('keeps no quoted and timestamps bare', () => {
    const c = records.find((x) => x.path.includes('03-choice'))!;
    const out = serializeRecord(parseRecord(c.text));
    expect(out).toContain('options: [once, always, "no"]');
    expect(out).toMatch(/^updated_at: 2026-10-24T18:05:30Z$/m);
    expect(out).toMatch(/^requests: \{delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null\}$/m);
  });

  it('reads schema 2 as pri 0 and writes schema 4 with pri', () => {
    const old = parseRecord(records.find((x) => x.path.includes('01-reminder'))!.text.replace(/^schema: 4$/m, 'schema: 2').replace(/^(pri|paused|suspended|suspend_until):.*\n/gm, ''));
    expect([old.schema, old.pri]).toEqual([2, 0]);
    const out = serializeRecord(old);
    expect(out).toMatch(/^schema: 6$/m);
    expect(out).toMatch(/^pri: 0$/m);
    expect(out.indexOf('\ndue:')).toBeLessThan(out.indexOf('\npri:'));
    expect(parseRecord(out)).toMatchObject({ schema: 6, pri: 0 });
  });

  it('keeps pri and a timed due through a round trip', () => {
    const r = parseRecord(records[0].text);
    const out = parseRecord(serializeRecord({ ...r, pri: 2, due: '2026-11-01T09:30' }));
    expect([out.pri, out.due]).toEqual([2, '2026-11-01T09:30']);
  });

  it('round-trips bound actions and omits an empty list', () => {
    const r = parseRecord(records[0].text);
    const withAction = {
      ...r,
      plan_meta: {
        reversibility: 'reversible' as const,
        tools: ['cartesia_speak'],
        requires: { tools: [], checks: [] },
        assumptions: [],
        checks: [],
        bound_actions: [{ tool: 'cartesia_speak', params: '{"path":"speech.wav"}', destination: 'speech.wav', effect: 'external_effect', grant_class: 'external_action' }],
      },
    };
    const out = serializeRecord(withAction);
    expect(out).toContain('bound_actions:');
    expect(out).toContain('tool: cartesia_speak');
    expect(parseRecord(out).plan_meta?.bound_actions).toEqual(withAction.plan_meta.bound_actions);
    const cleared = serializeRecord({ ...withAction, plan_meta: { ...withAction.plan_meta, bound_actions: [] } });
    expect(cleared).not.toContain('bound_actions');
  });

  it('refuses an unknown schema and a bad pri', () => {
    const text = records[0].text;
    expect(() => parseRecord(text.replace(/^schema: 4$/m, 'schema: 9'))).toThrow(/schema/);
    expect(() => parseRecord(serializeRecord(parseRecord(text)).replace(/^pri: 0$/m, 'pri: 9'))).toThrow(/pri/);
  });
});

describe('schema 4 fields', () => {
  it('reads a schema 3 record with the new defaults and writes it as schema 4', () => {
    const old = records.find((x) => x.path.includes('05-running'))!.text.replace(/^schema: 4$/m, 'schema: 3').replace(/^(paused|suspended|suspend_until):.*\n/gm, '');
    const r = parseRecord(old);
    expect(r).toMatchObject({ schema: 3, paused: false, suspended: false, suspend_until: null, requests: { pause: false, resume: false, complete: false } });
    const out = serializeRecord(r);
    expect(out).toMatch(/^schema: 6$/m);
    expect(out).toMatch(/^requests: \{delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null\}$/m);
    expect(out.indexOf('\nwaiting:')).toBeLessThan(out.indexOf('\npaused:'));
    expect(out.indexOf('\nsuspend_until:')).toBeLessThan(out.indexOf('\ncreated_at:'));
    expect(parseRecord(serializeRecord({ ...r, suspended: true, suspend_until: '2026-11-01T09:30' }))).toMatchObject({ suspended: true, suspend_until: '2026-11-01T09:30' });
  });
});
