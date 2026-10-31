import { describe, expect, it } from 'vitest';
import { shouldShowSetup } from './gate';
import { emptySettings } from './setup-source';

const done = () => ({ ...emptySettings(), setup: { completed_steps: ['folder', 'claude'], completed: true } });

describe('shouldShowSetup', () => {
  it('shows setup when no settings file exists', () => {
    expect(shouldShowSetup({ exists: false, settings: emptySettings() })).toBe(true);
  });
  it('shows setup when setup is not completed', () => {
    expect(shouldShowSetup({ exists: true, settings: emptySettings() })).toBe(true);
  });
  it('shows setup when the settings read failed', () => {
    expect(shouldShowSetup(null)).toBe(true);
  });
  it('shows setup again for old settings that have no Claude Code step', () => {
    const old = { ...emptySettings(), setup: { completed_steps: ['folder', 'api_key', 'model'], completed: true } };
    expect(shouldShowSetup({ exists: true, settings: old })).toBe(true);
  });
  it('skips setup when completed', () => {
    expect(shouldShowSetup({ exists: true, settings: done() })).toBe(false);
  });
});
