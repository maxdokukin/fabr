import { createApi } from '../api/index';
import { ApiError, errText, type CcSession, type TranscriptEntry } from '../api/types';
import { shellQuote, type OpenOutcome } from './sessions';

export function ago(iso: string, now: Date = new Date()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now.getTime() - t) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

export const newestFirst = (list: CcSession[]): CcSession[] => [...list].sort((a, b) => b.modified_at.localeCompare(a.modified_at));

export const sessionTitle = (s: { title: string }): string => s.title.trim() || 'Untitled';

export const toolLine = (e: TranscriptEntry): string => `${e.tool === 'error' ? 'Failed' : e.tool || 'Tool'}  ${e.text}`.trim();

export function nodeFolder(root: string, path: string, home: string): string {
  const base = root === '~' ? home : root.startsWith('~/') ? `${home}/${root.slice(2)}` : root;
  return [base.replace(/\/+$/, ''), path.replace(/^\/+/, '')].filter(Boolean).join('/');
}

export async function openNewClaude(folder: string): Promise<OpenOutcome> {
  try {
    await (await createApi()).fabricOpenClaude({ folder });
    return { opened: true };
  } catch (e) {
    if (e instanceof ApiError && e.code === 501) {
      try {
        await navigator.clipboard.writeText(`cd ${shellQuote(folder)} && claude`);
        return { opened: false, copied: true, message: 'Command copied. Paste it in a terminal.' };
      } catch {
        return { opened: false, copied: false, message: errText(e) };
      }
    }
    return { opened: false, copied: false, message: errText(e) };
  }
}
