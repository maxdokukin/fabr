import { buildApi, COMMANDS, type Api } from './commands';
import { ApiError, errText, type ErrorDto } from './types';

const BASE = '/api/';

function query(req: unknown): string {
  if (!req || typeof req !== 'object') return '';
  const parts = Object.entries(req as Record<string, unknown>)
    .filter(([, v]) => v !== undefined && v !== null)
    .map(([k, v]) => `${encodeURIComponent(k)}=${typeof v === 'boolean' ? (v ? 1 : 0) : encodeURIComponent(String(v))}`);
  return parts.length ? `?${parts.join('&')}` : '';
}

async function call(name: string, get: boolean, req: unknown): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(BASE + name + (get ? query(req) : ''), get
      ? { method: 'GET', headers: { Accept: 'application/json' } }
      : { method: 'POST', headers: { 'Content-Type': 'application/json', Accept: 'application/json' }, body: JSON.stringify(req ?? {}) });
  } catch (e) {
    throw new ApiError(0, `Network error: ${errText(e)}`);
  }
  const text = await res.text();
  let json: unknown = undefined;
  if (text) {
    try { json = JSON.parse(text); } catch {}
  }
  if (!res.ok) {
    const dto = json as Partial<ErrorDto> | undefined;
    if (dto && typeof dto === 'object' && typeof dto.message === 'string') throw ApiError.from(dto, res.status);
    throw new ApiError(res.status, text.trim() || `${name} failed (${res.status})`);
  }
  return json;
}

function realHttpApi(): Api {
  return buildApi(call, {
    fabricOpenSession: async () => {
      throw new ApiError(501, 'Opening a terminal works in the desktop app only');
    },
    fabricOpenClaude: async () => {
      throw new ApiError(501, 'Opening a terminal works in the desktop app only');
    },
    openUrl: async (url) => {
      if (/^https?:\/\//i.test(url)) window.open(url, '_blank', 'noopener');
    },
  });
}

function mockApi(): Api {
  let p: Promise<Api> | null = null;
  const get = () => (p ??= import('../../mock/api').then((m) => m.createMockApi()));
  const out: Record<string, unknown> = {};
  for (const n of Object.keys(COMMANDS) as (keyof Api)[]) {
    out[n] = async (...args: unknown[]) => ((await get())[n] as (...a: unknown[]) => Promise<unknown>)(...args);
  }
  return out as unknown as Api;
}

export function createHttpApi(): Api {
  if (import.meta.env.VITE_MOCK === '1') return mockApi();
  return realHttpApi();
}
