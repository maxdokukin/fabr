import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import SetupWizard from './SetupWizard.svelte';
import { emptySettings, StubSetupSource } from './setup-source';
import { firstIncompleteStep, STEPS, type StepId } from './wizard';

const ORDER = ['folder', 'claude'];
const HEADINGS: [StepId, string, number][] = [
  ['folder', 'Default folder', 0], ['claude', 'Claude Code', 1], ['done', 'All set', 2],
];

function settingsAt(n: number) {
  const s = emptySettings();
  s.setup = { completed_steps: ORDER.slice(0, n), completed: n === 2 };
  return s;
}

describe('SetupWizard render', () => {
  HEADINGS.forEach(([id, heading, n]) => {
    it(`${id} shows ${heading}`, () => {
      const initial = settingsAt(n);
      expect(firstIncompleteStep(initial)).toBe(id);
      const { body } = render(SetupWizard, { props: { source: new StubSetupSource({ settings: initial }), initial } });
      expect(body).toContain(heading);
      expect(body).toContain(`Step ${STEPS.indexOf(id) + 1} of 5`);
      expect(body.includes('>Back<')).toBe(n > 0);
    });
  });
  it('offers Open on the last screen', () => {
    const { body } = render(SetupWizard, { props: { initial: settingsAt(2), source: new StubSetupSource({ settings: settingsAt(2) }) } });
    expect(body).toContain('>Open<');
  });
  it('the Claude Code screen has the program, model and effort fields', () => {
    const initial = settingsAt(1);
    const { body } = render(SetupWizard, { props: { source: new StubSetupSource({ settings: initial }), initial } });
    for (const t of ['Program', 'Model', 'Effort', 'Sonnet', 'Fable', 'Extra high', '>Check<', '>Skip<', 'press Skip']) expect(body).toContain(t);
    expect(body).not.toContain('API');
  });
});

describe('stub source behaviour', () => {
  it('a scripted folder failure leaves the step incomplete and carries the reason', async () => {
    const stub = new StubSetupSource({ outcomes: { folder: { ok: false, reason: 'Folder is not writable.' } } });
    const r = await stub.check('folder', { path: '~/x' });
    expect(r).toEqual({ ok: false, reason: 'Folder is not writable.' });
    expect(firstIncompleteStep((await stub.readSettings()).settings)).toBe('folder');
  });
  it('a success marks the step and advances', async () => {
    const stub = new StubSetupSource();
    const { markStep } = await import('./wizard');
    const r = await stub.check('folder', { path: '~/' });
    expect(r.ok).toBe(true);
    const s = (await stub.readSettings()).settings;
    await stub.writeSettings(markStep({ ...s, default_folder: '~/' }, 'folder'));
    expect(firstIncompleteStep((await stub.readSettings()).settings)).toBe('claude');
  });
});
