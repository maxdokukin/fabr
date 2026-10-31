<script lang="ts">
  import { onMount } from 'svelte';
  import Group from '../ui/Group.svelte';
  import NumField from '../ui/NumField.svelte';
  import Row from '../ui/Row.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import ClaudeFields from '../setup/ClaudeFields.svelte';
  import Switch from '../ui/Switch.svelte';
  import type { ExecuteMode } from '../api/types';
  import type { SettingsController } from './controller.svelte';

  let { ctl }: { ctl: SettingsController } = $props();
  const p = $derived(ctl.setup);
  let bin = $state('');
  let model = $state('');
  let effort = $state('');
  let auto = $state(false);
  let replan = $state(true);
  let propose = $state(true);
  let execute = $state<ExecuteMode>('full');
  const MODES: { value: ExecuteMode; label: string }[] = [{ value: 'full', label: 'Full' }, { value: 'read_only', label: 'Read-only' }, { value: 'off', label: 'Off' }];
  const HELP: Record<ExecuteMode, string> = { full: 'This machine runs tasks to the end, with tools and checks.', read_only: 'This machine defines, plans and gates tasks but never executes them.', off: 'The agent does not run on this machine.' };

  onMount(async () => {
    await p.load();
    bin = p.settings?.claude_bin ?? '';
    model = p.settings?.model ?? '';
    effort = p.settings?.effort ?? '';
    auto = p.settings?.auto_mode ?? false;
    replan = p.settings?.replan_on_fail ?? true;
    propose = p.settings?.propose_procedures ?? true;
    execute = p.settings?.execute ?? 'full';
  });

  async function check() {
    await p.testClaude(bin);
    if (p.results.claude?.ok) await p.write({ claude_bin: bin.trim() });
  }
</script>

<Group header="Claude Code" footer="Fabr runs your tasks with the Claude Code program on this computer. Leave the program empty to find it automatically.">
  <ClaudeFields bind:bin bind:model bind:effort onbin={() => delete p.results.claude} onpick={() => p.write({ model, effort })} />
  <Row label={p.busy === 'claude' ? 'Checking…' : 'Check'} tone="tint" disabled={p.busy !== null} onclick={check} />
  {#if p.results.claude}
    <Row label="Result" wrap>
      <span class="row-result" class:row-result--bad={!p.results.claude.ok}><span class="row-dot" class:row-dot--ok={p.results.claude.ok} class:row-dot--error={!p.results.claude.ok}></span>{p.results.claude.text}</span>
    </Row>
    {#if p.results.claude.auth}<Row label="Signed in" value={p.results.claude.auth} />{/if}
  {/if}
</Group>

<Group header="Agent" footer={p.runnerError ?? 'The agent asks at most this many questions per task.'} error={!!p.runnerError}>
  <Row label="Auto mode"><Switch label="Auto mode" bind:checked={auto} onchange={(v) => p.write({ auto_mode: v })} /></Row>
  <NumField label="Question budget" mode="numeric" bad={!!p.runnerError} bind:value={p.budgetText} oncommit={() => p.saveRunner()} />
  <NumField label="Heartbeat" unit="seconds" mode="numeric" bad={!!p.runnerError} bind:value={p.heartbeatText} oncommit={() => p.saveRunner()} />
</Group>

<Group header="Limits" footer={p.limitsError ?? 'A run stops when it reaches the spend cap or the time limit.'} error={!!p.limitsError}>
  <NumField label="Spend cap per run (USD, 0 = none)" mode="decimal" bad={!!p.limitsError} bind:value={p.budgetUsdText} oncommit={() => p.saveLimits()} />
  <NumField label="Run time limit (minutes)" mode="numeric" bad={!!p.limitsError} bind:value={p.timeoutMinText} oncommit={() => p.saveLimits()} />
  <Row label="Re-plan once when the check fails"><Switch label="Re-plan once when the check fails" bind:checked={replan} onchange={(v) => p.write({ replan_on_fail: v })} /></Row>
  <Row label="Suggest reusable procedures after a task passes"><Switch label="Suggest reusable procedures after a task passes" bind:checked={propose} onchange={(v) => p.write({ propose_procedures: v })} /></Row>
</Group>

<Group header="Machine" footer="{HELP[execute]} The runner detects tools and checks and refreshes them when it starts.">
  <Row label="Execute"><Segmented label="Execute" options={MODES} bind:value={execute} onchange={(m) => p.write({ execute: m })} /></Row>
  <Row label="Tools" wrap>
    <span class="chips" data-testid="cap-tools">{#each p.settings?.capabilities.tools ?? [] as t (t)}<span class="chip">{t}</span>{:else}<span class="chip chip--none">none detected</span>{/each}</span>
  </Row>
  <Row label="Checks" wrap>
    <span class="chips" data-testid="cap-checks">{#each p.settings?.capabilities.checks ?? [] as c (c)}<span class="chip">{c}</span>{:else}<span class="chip chip--none">none detected</span>{/each}</span>
  </Row>
</Group>

{#if p.error}<div class="banner banner--error page-panel" role="alert">{p.error}</div>{/if}

<style>
  .chips { display: flex; flex-wrap: wrap; gap: 4px; min-width: 0; }
  .chip { padding: 1px 8px; border-radius: 999px; background: var(--fill-3); font-size: var(--fs-12); overflow-wrap: anywhere; }
  .chip--none { color: var(--label-2); }
</style>
