import { describe, expect, it } from 'vitest';
import { canGoBack, canSkip, firstIncompleteStep, markStep, next, prev, setupComplete, skipStep, skipTo, stepIndex } from './wizard';
import { emptySettings } from './setup-source';

const withSteps = (steps: string[], completed = false) => ({ ...emptySettings(), setup: { completed_steps: steps, completed } });

describe('wizard', () => {
  it('opens the first incomplete required step', () => {
    expect(firstIncompleteStep(emptySettings())).toBe('folder');
    expect(firstIncompleteStep(withSteps(['folder']))).toBe('claude');
    expect(firstIncompleteStep(withSteps(['claude']))).toBe('folder');
    expect(firstIncompleteStep(withSteps(['folder', 'claude']))).toBe('done');
  });
  it('old settings with api steps show the Claude Code screen', () => {
    const old = withSteps(['folder', 'api_key', 'model', 'telegram_bot', 'telegram_link'], true);
    expect(firstIncompleteStep(old)).toBe('claude');
    expect(setupComplete(old)).toBe(false);
    expect(setupComplete(withSteps(['folder', 'claude'], true))).toBe(true);
  });
  it('navigates', () => {
    expect(canGoBack('folder')).toBe(false);
    expect(canGoBack('claude')).toBe(true);
    expect(next('folder')).toBe('claude');
    expect(next('done')).toBe('done');
    expect(prev('folder')).toBe('folder');
    expect(prev('done')).toBe('telegram_link');
    expect(stepIndex('done')).toBe(4);
  });
  it('every screen but the folder can be skipped', () => {
    expect(['folder', 'claude', 'telegram_bot', 'telegram_link', 'done'].filter((s) => canSkip(s as never))).toEqual(['claude', 'telegram_bot', 'telegram_link']);
    expect([skipTo('claude'), skipTo('telegram_bot'), skipTo('telegram_link')]).toEqual(['telegram_bot', 'done', 'done']);
  });
  it('a skipped Claude Code step completes setup without counting as done', () => {
    const s = skipStep(markStep(emptySettings(), 'folder'), 'claude');
    expect(s.setup).toEqual({ completed_steps: ['folder', 'claude_skipped'], completed: true });
    expect(setupComplete(s)).toBe(true);
    expect(firstIncompleteStep(s)).toBe('done');
    expect(skipStep(s, 'claude')).toBe(s);
    expect(markStep(s, 'claude').setup.completed_steps).toEqual(['folder', 'claude']);
    expect(skipStep(markStep(s, 'claude'), 'claude').setup.completed_steps).toEqual(['folder', 'claude']);
  });
  it('completes once folder and claude are done', () => {
    let s = markStep(emptySettings(), 'folder');
    expect(s.setup.completed).toBe(false);
    s = markStep(markStep(s, 'claude'), 'claude');
    expect(s.setup.completed).toBe(true);
    expect(s.setup.completed_steps).toEqual(['folder', 'claude']);
  });
});
