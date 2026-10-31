import { createApi } from '../api/index';
import { ApiError, errText } from '../api/types';
import type { Sessions } from './types';

const LABELS: [keyof Sessions, string][] = [['plan', 'Planner'], ['execute', 'Executor'], ['evaluate', 'Validator']];
const PLAIN = /^[\w./~@%+=:,-]+$/;

export const shellQuote = (s: string): string => (PLAIN.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);

export function sessionList(s: Sessions): { key: keyof Sessions; label: string; id: string }[] {
  return LABELS.flatMap(([key, label]) => (s[key] ? [{ key, label, id: s[key] as string }] : []));
}

export const sessionCommand = (folder: string, id: string): string => `cd ${shellQuote(folder)} && claude --resume ${id}`;

export async function copyCommand(folder: string, id: string): Promise<string> {
  const cmd = sessionCommand(folder, id);
  await navigator.clipboard.writeText(cmd);
  return cmd;
}

export type OpenOutcome = { opened: true } | { opened: false; copied: boolean; message: string };

export async function openSession(folder: string, id: string): Promise<OpenOutcome> {
  try {
    await (await createApi()).fabricOpenSession({ folder, session_id: id });
    return { opened: true };
  } catch (e) {
    if (e instanceof ApiError && e.code === 501) {
      try {
        await copyCommand(folder, id);
        return { opened: false, copied: true, message: 'Command copied. Paste it in a terminal.' };
      } catch {
        return { opened: false, copied: false, message: errText(e) };
      }
    }
    return { opened: false, copied: false, message: errText(e) };
  }
}
