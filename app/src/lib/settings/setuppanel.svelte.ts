import { errText, type FabricCheckResult, type FabricSecretsStatus, type FabricSettings } from '../api/types';
import { createSetupSource, type SetupSource } from '../setup/setup-source';

export interface Outcome { ok: boolean; text: string; auth?: string }
export type Probe = 'claude' | 'token' | 'link';

const NO_SECRETS: FabricSecretsStatus = { telegram_bot_token: false };
const LINK_TIMEOUT_S = 120;

export function parseWhole(text: string, min: number, max: number): number | null {
  const t = text.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return n >= min && n <= max ? n : null;
}

export function parseUsd(text: string): number | null {
  const t = text.trim();
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return null;
  const n = Number(t);
  return n <= 100000 ? n : null;
}

export class SetupPanel {
  settings = $state<FabricSettings | null>(null);
  secrets = $state<FabricSecretsStatus>({ ...NO_SECRETS });
  loaded = $state(false);
  error = $state<string | null>(null);
  busy = $state<Probe | 'save' | null>(null);
  results = $state<Partial<Record<Probe, Outcome>>>({});
  elapsed = $state(0);
  budgetText = $state('');
  heartbeatText = $state('');
  runnerError = $state<string | null>(null);
  budgetUsdText = $state('0');
  timeoutMinText = $state('30');
  limitsError = $state<string | null>(null);

  private src: SetupSource | null = null;
  private factory: () => Promise<SetupSource>;
  private timer: ReturnType<typeof setInterval> | undefined;

  constructor(factory: () => Promise<SetupSource> = () => createSetupSource()) {
    this.factory = factory;
  }

  get claudeReady(): boolean {
    return !!this.settings?.setup.completed_steps.includes('claude');
  }
  get tokenSaved(): boolean {
    return this.secrets.telegram_bot_token;
  }
  get linkedId(): number | null {
    return this.settings?.telegram_user_id ?? null;
  }
  get telegramReady(): boolean {
    return this.tokenSaved && this.linkedId !== null;
  }
  get tokenFingerprint(): string {
    return this.secrets.telegram_bot_token_fingerprint ?? 'saved';
  }

  clearResults(): void {
    this.results = {};
    this.error = null;
    this.runnerError = null;
    this.limitsError = null;
  }

  async load(): Promise<void> {
    if (this.loaded) return;
    try {
      this.src ??= await this.factory();
      this.secrets = await this.src.secretsStatus();
      this.adopt((await this.src.readSettings()).settings);
      this.loaded = true;
      this.error = null;
    } catch (e) {
      this.error = errText(e);
    }
  }

  private adopt(s: FabricSettings): void {
    this.settings = s;
    this.budgetText = String(s.question_budget);
    this.heartbeatText = String(s.heartbeat_interval_s);
    this.budgetUsdText = String(s.max_budget_usd);
    this.timeoutMinText = String(Math.round(s.execute_timeout_s / 60));
  }

  private async guard(work: () => Promise<void>): Promise<boolean> {
    try {
      await work();
      this.error = null;
      return true;
    } catch (e) {
      this.error = errText(e);
      return false;
    }
  }

  async write(patch: Partial<FabricSettings>): Promise<boolean> {
    if (!this.src || !this.settings) return false;
    const src = this.src;
    const next = { ...$state.snapshot(this.settings) as FabricSettings, ...patch };
    this.busy = 'save';
    const ok = await this.guard(async () => this.adopt((await src.writeSettings(next)).settings));
    this.busy = null;
    return ok;
  }

  async saveSecret(value: string): Promise<boolean> {
    if (!this.src || !value.trim()) return false;
    const src = this.src;
    this.busy = 'save';
    const ok = await this.guard(async () => {
      this.secrets = await src.writeSecrets({ telegram_bot_token: value });
    });
    if (ok) delete this.results.token;
    this.busy = null;
    return ok;
  }

  private async probe(kind: Probe, work: () => Promise<FabricCheckResult>, good: (r: Extract<FabricCheckResult, { ok: true }>) => string): Promise<void> {
    if (!this.src || this.busy) return;
    this.busy = kind;
    delete this.results[kind];
    try {
      const r = await work();
      this.results[kind] = r.ok ? { ok: true, text: good(r), auth: r.auth } : { ok: false, text: r.reason };
    } catch (e) {
      this.results[kind] = { ok: false, text: errText(e) };
    }
    this.busy = null;
  }

  testClaude(bin: string): Promise<void> {
    return this.probe('claude', () => this.src!.check('claude', { path: bin.trim() }), (r) => [r.version, r.path].filter(Boolean).join(' at ') || 'Claude Code works');
  }

  testToken(): Promise<void> {
    return this.probe('token', () => this.src!.check('telegram_token'), (r) => (r.bot_name ? `Bot ${r.bot_name}` : 'Token works'));
  }

  async link(): Promise<void> {
    if (!this.src || this.busy) return;
    this.busy = 'link';
    delete this.results.link;
    this.elapsed = 0;
    this.timer = setInterval(() => (this.elapsed += 1), 1000);
    try {
      const r = await this.src.telegramLink(LINK_TIMEOUT_S);
      if (r.ok && r.user_id != null) {
        this.busy = null;
        if (await this.write({ telegram_user_id: r.user_id })) this.results.link = { ok: true, text: `Linked as user ${r.user_id}` };
      } else {
        this.results.link = { ok: false, text: r.ok ? 'No user id came back.' : r.reason };
      }
    } catch (e) {
      this.results.link = { ok: false, text: errText(e) };
    }
    clearInterval(this.timer);
    this.busy = null;
  }

  async unlink(): Promise<void> {
    delete this.results.link;
    await this.write({ telegram_user_id: null });
  }

  async saveRunner(): Promise<boolean> {
    const budget = parseWhole(this.budgetText, 0, 100);
    const beat = parseWhole(this.heartbeatText, 1, 3600);
    this.runnerError = budget === null ? 'Question budget must be a whole number from 0 to 100.' : beat === null ? 'Heartbeat must be a whole number of seconds from 1 to 3600.' : null;
    if (budget === null || beat === null || !this.settings) return false;
    if (budget === this.settings.question_budget && beat === this.settings.heartbeat_interval_s) return true;
    return this.write({ question_budget: budget, heartbeat_interval_s: beat });
  }

  async saveLimits(): Promise<boolean> {
    const usd = parseUsd(this.budgetUsdText);
    const min = parseWhole(this.timeoutMinText, 1, 240);
    this.limitsError = usd === null ? 'Spend cap must be an amount in USD, 0 for none.' : min === null ? 'Run time limit must be a whole number of minutes from 1 to 240.' : null;
    if (usd === null || min === null || !this.settings) return false;
    if (usd === this.settings.max_budget_usd && min * 60 === this.settings.execute_timeout_s) return true;
    return this.write({ max_budget_usd: usd, execute_timeout_s: min * 60 });
  }
}
