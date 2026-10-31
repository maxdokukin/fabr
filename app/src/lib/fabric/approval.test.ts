import { describe, expect, it } from 'vitest';
import { applyApproval, needsHumanApproval, submitApproval } from './approval';
import { parseRecord } from './parse';
import { serializeRecord } from './serialize';
import { FixtureSource } from './source';
import { ConflictError } from './write';

const src = new FixtureSource();
const records = await src.listRecords();
const entry = (n: string) => records.find((r) => r.path.includes(n))!;
const pending = parseRecord(entry('04-awaiting').text);
const now = new Date('2026-10-24T19:00:00.500Z');

function diff(a: unknown, b: unknown, path = ''): string[] {
  if (a !== null && b !== null && typeof a === 'object' && typeof b === 'object' && !Array.isArray(a)) {
    const keys = new Set([...Object.keys(a as object), ...Object.keys(b as object)]);
    return [...keys].flatMap((k) => diff((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k], path ? `${path}.${k}` : k));
  }
  return path === 'schema' || JSON.stringify(a) === JSON.stringify(b) ? [] : [path];
}

describe('applyApproval', () => {
  it('approving two of three fields at mixed scopes touches only approval, grants.granted and updated_at', () => {
    const r = applyApproval(pending, { status: 'approved', granted: [{ field: 'address.line_1', scope: 'always' }, { field: 'name.first' }] }, 'gui', now);
    const after = parseRecord(serializeRecord(r));
    expect(after.grants.granted).toEqual([{ field: 'address.line_1', scope: 'always' }, { field: 'name.first', scope: 'task' }]);
    expect(after.approval).toEqual({ status: 'approved', by: 'gui', at: '2026-10-24T19:00:00Z', note: null, digest: pending.approval?.digest ?? null });
    const changed = diff(pending, after);
    expect(changed.every((p) => p === 'updated_at' || p.startsWith('approval') || p === 'grants.granted')).toBe(true);
    expect(changed).toContain('updated_at');
    expect(pending.approval?.status).toBe('pending');
    expect(pending.approval?.digest).toMatch(/^[0-9a-f]{64}$/);
  });

  it('refusing with a note touches only approval and updated_at and keeps granted', () => {
    const r = applyApproval(pending, { status: 'refused', note: ' too risky ', granted: [{ field: 'name.first' }] }, 'gui', now);
    const after = parseRecord(serializeRecord(r));
    expect(after.approval).toEqual({ status: 'refused', by: 'gui', at: '2026-10-24T19:00:00Z', note: 'too risky', digest: pending.approval?.digest ?? null });
    const changed = diff(pending, after);
    expect(changed.every((p) => p === 'updated_at' || p.startsWith('approval'))).toBe(true);
    expect(after.grants.granted).toEqual([]);
  });

  it('rejects fields that were not requested, bad scopes and duplicates', () => {
    expect(() => applyApproval(pending, { status: 'approved', granted: [{ field: 'card.number' }] }, 'gui')).toThrow(/not requested/);
    expect(() => applyApproval(pending, { status: 'approved', granted: [{ field: 'name.first', scope: 'forever' as never }] }, 'gui')).toThrow(/scope/);
    expect(() => applyApproval(pending, { status: 'approved', granted: [{ field: 'name.first' }, { field: 'name.first' }] }, 'gui')).toThrow(/twice/);
  });

  it('throws when approval is null or already decided', () => {
    const none = parseRecord(entry('02-open').text);
    expect(() => applyApproval(none, { status: 'approved' }, 'gui')).toThrow(/no approval/);
    const decided = parseRecord(entry('05-running').text);
    expect(() => applyApproval(decided, { status: 'refused' }, 'gui')).toThrow(/already decided/);
    const once = applyApproval(pending, { status: 'approved', granted: [] }, 'gui', now);
    expect(() => applyApproval(once, { status: 'refused' }, 'gui')).toThrow(/already decided/);
  });
});

describe('needsHumanApproval', () => {
  it('is true for the irreversible fixture and false for fixture 02', () => {
    expect(needsHumanApproval(pending)).toBe(true);
    expect(needsHumanApproval(parseRecord(entry('02-open').text))).toBe(false);
  });

  it('follows payment fields and the always-ask list', () => {
    const base = { ...pending, plan_meta: { ...pending.plan_meta!, reversibility: 'reversible' as const } };
    expect(needsHumanApproval(base)).toBe(false);
    expect(needsHumanApproval(base, ['zip'])).toBe(false);
    expect(needsHumanApproval(base, ['address.zip'])).toBe(true);
    expect(needsHumanApproval({ ...base, grants: { requested: ['payment.card'], granted: [] } })).toBe(true);
  });
});

describe('submitApproval', () => {
  it('writes through the source and a second decision from the stale text conflicts', async () => {
    const s = new FixtureSource();
    const { path, text } = (await s.listRecords()).find((r) => r.path.includes('04-awaiting'))!;
    const out = await submitApproval(s, path, text, { status: 'approved', granted: [{ field: 'name.first', scope: 'project' }] });
    expect(out.record.approval!.status).toBe('approved');
    expect(parseRecord(await s.readRecord(path)).grants.granted).toEqual([{ field: 'name.first', scope: 'project' }]);
    await expect(submitApproval(s, path, out.text, { status: 'refused' })).rejects.toThrow(/already decided/);
    await expect(submitApproval(s, path, text, { status: 'refused' })).rejects.toBeInstanceOf(ConflictError);
  });
});

describe('replan notice', async () => {
  const { render } = await import('svelte/server');
  const RecordScreen = (await import('./RecordScreen.svelte')).default;
  const html = (rec: typeof pending) => render(RecordScreen, { props: { record: rec, name: 'x', now, onapprove: async () => {} } }).body;
  const log = (step: string, message: string) => ({ ts: '2026-10-24T19:00:00Z', step, message });
  const replanned = {
    ...pending,
    waiting: true,
    verdict: { result: 'fail' as const, at: '2026-10-24T18:59:00Z', items: [{ id: 'a1', pass: true, evidence: 'ok' }, { id: 'a2', pass: false, evidence: 'greet.md is empty' }] },
    sections: { ...pending.sections, logEntries: [...pending.sections.logEntries, log('evaluate', 'verdict fail'), log('evaluate', 'replanning after failed check (a2)'), log('plan', 'redrafted after failed check')] },
  };

  it('shows the failed items above a pending approval after a replan', () => {
    const body = html(replanned);
    expect(body).toContain('data-testid="replan-notice"');
    expect(body).toContain('The first attempt failed a check. Here is a revised plan.');
    expect(body).toMatch(/<b>a2<\/b> greet.md is empty/);
    expect(body.indexOf('replan-notice')).toBeLessThan(body.indexOf('Approval needed'));
    expect(body.slice(body.indexOf('replan-notice'), body.indexOf('Approval needed'))).not.toContain('<b>a1</b>');
  });

  it('labels the kept verdict as the first attempt until the task ends', () => {
    expect(html(replanned)).toContain('Verdict of the first attempt');
    expect(html({ ...replanned, state: 'succeeded' })).not.toContain('Verdict of the first attempt');
  });

  it('is absent without the marker or once the approval is decided', () => {
    expect(html({ ...pending, waiting: true })).not.toContain('replan-notice');
    expect(html({ ...replanned, waiting: false, approval: { ...replanned.approval!, status: 'approved' } })).not.toContain('replan-notice');
  });
});

describe('file rule chips', async () => {
  const { render } = await import('svelte/server');
  const ApprovalPanel = (await import('./ApprovalPanel.svelte')).default;
  const chips = (tools: string[]) => {
    const planMeta = { ...pending.plan_meta!, tools };
    return render(ApprovalPanel, { props: { approval: pending.approval!, grants: pending.grants, planMeta, plan: '', assertions: [] } }).body;
  };
  const chip = (body: string, rule: string) => body.slice(body.indexOf(`>${rule}<`) - 30, body.indexOf('</span>', body.indexOf(`>${rule}<`)));

  it('marks bare and relative file tools as inside this folder, without the warning highlight', () => {
    const body = chips(['Write', 'Edit(src/**)', 'MultiEdit', 'NotebookEdit']);
    for (const r of ['Write', 'Edit(src/**)', 'MultiEdit', 'NotebookEdit']) {
      expect(chip(body, r)).toContain('inside this folder');
      expect(chip(body, r)).not.toContain('hot');
    }
  });

  it('marks absolute, home and parent paths as outside this folder with the highlight', () => {
    const body = chips(['Edit(//etc/hosts)', 'Write(/tmp/x)', 'Edit(~/notes.md)', 'Write(../other/**)']);
    for (const r of ['Edit(//etc/hosts)', 'Write(/tmp/x)', 'Edit(~/notes.md)', 'Write(../other/**)']) {
      expect(chip(body, r)).toContain('outside this folder');
      expect(chip(body, r)).toContain('hot');
    }
  });
});
