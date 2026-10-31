import type { FabricSettings } from '../api/types';

export type StepId = 'folder' | 'claude' | 'telegram_bot' | 'telegram_link' | 'done';

export const STEPS: StepId[] = ['folder', 'claude', 'telegram_bot', 'telegram_link', 'done'];
export const REQUIRED_STEPS: StepId[] = ['folder', 'claude'];
export const OPTIONAL_STEPS: StepId[] = ['telegram_bot', 'telegram_link'];

export const MODELS = [
  { value: '', label: 'Default' },
  { value: 'sonnet', label: 'Sonnet' },
  { value: 'opus', label: 'Opus' },
  { value: 'haiku', label: 'Haiku' },
  { value: 'fable', label: 'Fable' },
];

export const EFFORTS = [
  { value: '', label: 'Default' },
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
  { value: 'xhigh', label: 'Extra high' },
  { value: 'max', label: 'Max' },
];

export const stepIndex = (s: StepId) => STEPS.indexOf(s);
export const canGoBack = (s: StepId) => stepIndex(s) > 0;
export const canSkip = (s: StepId) => OPTIONAL_STEPS.includes(s);
export const prev = (s: StepId): StepId => STEPS[Math.max(0, stepIndex(s) - 1)];
export const next = (s: StepId): StepId => STEPS[Math.min(STEPS.length - 1, stepIndex(s) + 1)];

const stepsOf = (settings: FabricSettings) => settings.setup?.completed_steps ?? [];

export function firstIncompleteStep(settings: FabricSettings): StepId {
  const done = stepsOf(settings);
  return REQUIRED_STEPS.find((s) => !done.includes(s)) ?? 'done';
}

export function setupComplete(settings: FabricSettings): boolean {
  return !!settings.setup?.completed && REQUIRED_STEPS.every((s) => stepsOf(settings).includes(s));
}

export function markStep(settings: FabricSettings, step: StepId): FabricSettings {
  const have = stepsOf(settings);
  const steps = have.includes(step) ? have : [...have, step];
  const completed = REQUIRED_STEPS.every((s) => steps.includes(s));
  return { ...settings, setup: { completed_steps: steps, completed } };
}
