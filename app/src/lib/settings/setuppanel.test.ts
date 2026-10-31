import { describe, expect, it } from 'vitest';
import { StubSetupSource, emptySettings } from '../setup/setup-source';
import { SetupPanel } from './setuppanel.svelte';

async function panel(opts: ConstructorParameters<typeof StubSetupSource>[0] = {}) {
  const src = new StubSetupSource(opts);
  const p = new SetupPanel(async () => src);
  await p.load();
  return { src, p };
}

describe('setup panel', () => {
  it('checks Claude Code and reports version and path', async () => {
    const { p } = await panel();
    expect(p.claudeReady).toBe(false);
    await p.testClaude('');
    expect(p.results.claude).toEqual({ ok: true, text: '2.1.287 (Claude Code) at /usr/local/bin/claude' });
  });

  it('reports a failed Claude Code check', async () => {
    const { p } = await panel({ outcomes: { claude: { ok: false, reason: 'Claude Code is not installed.' } } });
    await p.testClaude('');
    expect(p.results.claude).toEqual({ ok: false, text: 'Claude Code is not installed.' });
  });

  it('keeps the model, effort and program in settings', async () => {
    const { src, p } = await panel();
    expect(await p.write({ model: 'opus', effort: 'high', claude_bin: '/opt/claude' })).toBe(true);
    expect(src.settings).toMatchObject({ model: 'opus', effort: 'high', claude_bin: '/opt/claude' });
  });

  it('links, then unlinks the telegram user', async () => {
    const { src, p } = await panel();
    await p.saveSecret('123456:FAKE-FIXTURE-TOKEN-0000');
    await p.link();
    expect(p.linkedId).toBe(123456789);
    expect(src.settings.telegram_user_id).toBe(123456789);
    expect(p.telegramReady).toBe(true);
    await p.unlink();
    expect(src.settings.telegram_user_id).toBeNull();
    expect(p.linkedId).toBeNull();
  });

  it('validates runner numbers before writing', async () => {
    const { src, p } = await panel();
    p.budgetText = 'x';
    expect(await p.saveRunner()).toBe(false);
    expect(p.runnerError).toBeTruthy();
    p.budgetText = '7';
    p.heartbeatText = '30';
    expect(await p.saveRunner()).toBe(true);
    expect(src.settings).toMatchObject({ question_budget: 7, heartbeat_interval_s: 30 });
  });
});

describe('machine settings', () => {
  it('writes execute through the settings source and keeps the detected capabilities', async () => {
    const { src, p } = await panel({ settings: { ...emptySettings(), capabilities: { tools: ['write_file'], checks: ['git'] } } });
    expect(await p.write({ execute: 'read_only' })).toBe(true);
    expect(src.settings.execute).toBe('read_only');
    expect(src.settings.capabilities).toEqual({ tools: ['write_file'], checks: ['git'] });
    expect(p.settings?.execute).toBe('read_only');
  });

  it('renders the three modes and the capability chips', async () => {
    const { render } = await import('svelte/server');
    const ClaudePage = (await import('./ClaudePage.svelte')).default;
    const { p } = await panel({ settings: { ...emptySettings(), capabilities: { tools: ['write_file'], checks: ['git'] } } });
    const body = render(ClaudePage, { props: { ctl: { setup: p } as never } }).body;
    for (const t of ['Claude Code', 'Effort', 'Full', 'Read-only', 'Off', 'write_file', 'git', 'refreshes them when it starts']) expect(body).toContain(t);
  });
});
