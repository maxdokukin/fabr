import { createApi } from '../api/index';
import type { FabricCheckKind, FabricCheckResult, FabricSecretsStatus, FabricSecretsWriteReq, FabricSettings, FabricSettingsRead } from '../api/types';

export interface CheckArgs { path?: string }

export interface SetupSource {
  readSettings(): Promise<FabricSettingsRead>;
  writeSettings(settings: FabricSettings): Promise<FabricSettingsRead>;
  writeSecrets(partial: FabricSecretsWriteReq): Promise<FabricSecretsStatus>;
  secretsStatus(): Promise<FabricSecretsStatus>;
  check(kind: FabricCheckKind, args?: CheckArgs): Promise<FabricCheckResult>;
  telegramLink(timeoutS: number): Promise<FabricCheckResult>;
}

type Api = Awaited<ReturnType<typeof createApi>>;

export class ApiSetupSource implements SetupSource {
  constructor(private api: Api) {}
  readSettings() { return this.api.fabricSettingsRead(); }
  writeSettings(settings: FabricSettings) { return this.api.fabricSettingsWrite(settings); }
  writeSecrets(partial: FabricSecretsWriteReq) { return this.api.fabricSecretsWrite(partial); }
  secretsStatus() { return this.api.fabricSecretsStatus(); }
  check(kind: FabricCheckKind, args: CheckArgs = {}) { return this.api.fabricCheck({ kind, ...args }); }
  telegramLink(timeoutS: number) { return this.api.fabricTelegramLink(timeoutS); }
}

export function fingerprint(key: string): string {
  const c = [...key];
  return c.length < 20 ? '…' : `${c.slice(0, 7).join('')}…${c.slice(-4).join('')}`;
}

export type StubKey = FabricCheckKind | 'telegram_link';

export function emptySettings(): FabricSettings {
  return {
    default_folder: '', model: '', telegram_user_id: null, auto_mode: false,
    question_budget: 5, heartbeat_interval_s: 10, execute: 'full', capabilities: { tools: [], checks: [] }, setup: { completed_steps: [], completed: false },
    claude_bin: '', effort: '',
  };
}

const OK: Record<StubKey, FabricCheckResult> = {
  folder: { ok: true },
  claude: { ok: true, version: '2.1.287 (Claude Code)', path: '/usr/local/bin/claude' },
  telegram_token: { ok: true, bot_name: 'test_bot' },
  telegram_link: { ok: true, user_id: 123456789 },
};

export class StubSetupSource implements SetupSource {
  settings: FabricSettings;
  exists: boolean;
  secrets: FabricSecretsStatus = { telegram_bot_token: false };
  outcomes: Partial<Record<StubKey, FabricCheckResult>>;
  delayMs: number;
  calls: string[] = [];

  constructor(opts: { settings?: FabricSettings; outcomes?: Partial<Record<StubKey, FabricCheckResult>>; delayMs?: number } = {}) {
    this.settings = structuredClone(opts.settings ?? emptySettings());
    this.exists = !!opts.settings;
    this.outcomes = opts.outcomes ?? {};
    this.delayMs = opts.delayMs ?? 0;
  }

  private wait() { return this.delayMs ? new Promise((r) => setTimeout(r, this.delayMs)) : Promise.resolve(); }
  private snapshot(): FabricSettingsRead { return { exists: this.exists, settings: structuredClone(this.settings) }; }

  async readSettings() { this.calls.push('readSettings'); return this.snapshot(); }
  async writeSettings(settings: FabricSettings) {
    this.calls.push('writeSettings');
    this.settings = structuredClone(settings);
    this.exists = true;
    return this.snapshot();
  }
  async writeSecrets(partial: FabricSecretsWriteReq) {
    this.calls.push('writeSecrets');
    if (partial.telegram_bot_token) {
      this.secrets.telegram_bot_token = true;
      this.secrets.telegram_bot_token_fingerprint = fingerprint(partial.telegram_bot_token.trim());
    }
    return { ...this.secrets };
  }
  async secretsStatus() { return { ...this.secrets }; }
  async check(kind: FabricCheckKind, _args: CheckArgs = {}) {
    this.calls.push(`check:${kind}`);
    await this.wait();
    return this.outcomes[kind] ?? OK[kind];
  }
  async telegramLink(_timeoutS: number) {
    this.calls.push('telegramLink');
    await this.wait();
    return this.outcomes.telegram_link ?? OK.telegram_link;
  }
}

export function stubFromQuery(search: string): StubSetupSource | null {
  const fail = new URLSearchParams(search).get('fail');
  if (!fail) return null;
  const outcomes: Partial<Record<StubKey, FabricCheckResult>> = {};
  for (const k of fail.split(',')) {
    if (k in OK) outcomes[k as StubKey] = { ok: false, reason: `Scripted failure for ${k}. Fix it and try again.` };
  }
  return new StubSetupSource({ outcomes, delayMs: 800 });
}

export async function createSetupSource(search = ''): Promise<SetupSource> {
  if (import.meta.env.DEV) {
    const stub = stubFromQuery(search);
    if (stub) return stub;
  }
  return new ApiSetupSource(await createApi());
}
