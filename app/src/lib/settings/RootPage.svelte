<script lang="ts">
  import { onMount } from 'svelte';
  import type { Store } from '../store.svelte';
  import Group from '../ui/Group.svelte';
  import NavRow from '../ui/NavRow.svelte';
  import { APP_NAME, APP_VERSION } from './about';
  import type { SettingsController } from './controller.svelte';

  let { store, ctl }: { store: Store; ctl: SettingsController } = $props();
  const n = $derived(ctl.drafts.length);
  const setup = $derived(ctl.setup);
  onMount(() => { void setup.load(); });
</script>

<Group>
  <NavRow label="Appearance" onclick={() => ctl.push({ kind: 'appearance' })} />
  <NavRow label="Sources" value={`${n} ${n === 1 ? 'source' : 'sources'}`} onclick={() => ctl.push({ kind: 'sources' })} />
  <NavRow label="Advanced" onclick={() => ctl.push({ kind: 'advanced' })} />
</Group>
<Group>
  <NavRow label="Claude Code" onclick={() => ctl.push({ kind: 'claude' })}>
    {#snippet lead()}<span class="row-dot" class:row-dot--ok={setup.claudeReady}></span>{/snippet}
  </NavRow>
  <NavRow label="Telegram" onclick={() => ctl.push({ kind: 'telegram' })}>
    {#snippet lead()}<span class="row-dot" class:row-dot--ok={setup.telegramReady}></span>{/snippet}
  </NavRow>
</Group>
<Group>
  <NavRow label="Keyboard shortcuts" onclick={() => ctl.push({ kind: 'keys' })} />
</Group>
<p class="settings-about">
  {APP_NAME} {APP_VERSION}
  {#if store.settings.log_path}<br /><span class="settings-about__path">{store.settings.log_path}</span>{/if}
</p>
