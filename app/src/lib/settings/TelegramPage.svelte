<script lang="ts">
  import { onMount } from 'svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';
  import type { SettingsController } from './controller.svelte';

  let { ctl }: { ctl: SettingsController } = $props();
  const p = $derived(ctl.setup);
  let secret = $state('');

  onMount(() => { void p.load(); });

  async function save() {
    if (await p.saveSecret(secret)) secret = '';
  }
</script>

<Group header="Bot token" footer="Stored on this computer. Only the start and end of the token are shown.">
  <Row label="Saved token" value={p.tokenSaved ? p.tokenFingerprint : 'none'} />
  <Row label="Replace with">
    <input class="row-input" type="password" bind:value={secret} placeholder="123456:ABC..." autocomplete="off" spellcheck="false" />
  </Row>
  <Row label={p.busy === 'save' ? 'Saving…' : 'Save token'} tone="tint" disabled={!secret.trim() || p.busy !== null} onclick={save} />
  <Row label={p.busy === 'token' ? 'Testing…' : 'Test token'} tone="tint" disabled={!p.tokenSaved || p.busy !== null} onclick={() => p.testToken()} />
  {#if p.results.token}
    <Row label="Result" wrap>
      <span class="row-result" class:row-result--bad={!p.results.token.ok}><span class="row-dot" class:row-dot--ok={p.results.token.ok} class:row-dot--error={!p.results.token.ok}></span>{p.results.token.text}</span>
    </Row>
  {/if}
</Group>

<Group header="Linked account" footer={p.busy === 'link' ? 'Open your bot in Telegram, send it any message, then tap Confirm on its reply.' : undefined}>
  <Row label="User id" value={p.linkedId === null ? 'not linked' : String(p.linkedId)} />
  <Row label={p.busy === 'link' ? 'Waiting for your message…' : p.linkedId === null ? 'Link' : 'Link again'} tone="tint" disabled={!p.tokenSaved || p.busy !== null} onclick={() => p.link()} />
  {#if p.busy === 'link'}<Row label="Waiting" value={`${p.elapsed} s`} />{/if}
  {#if p.results.link}
    <Row label="Result" wrap>
      <span class="row-result" class:row-result--bad={!p.results.link.ok}><span class="row-dot" class:row-dot--ok={p.results.link.ok} class:row-dot--error={!p.results.link.ok}></span>{p.results.link.text}</span>
    </Row>
  {/if}
  {#if p.linkedId !== null}<Row label="Unlink" tone="danger" disabled={p.busy !== null} onclick={() => p.unlink()} />{/if}
</Group>

{#if p.error}<div class="banner banner--error page-panel" role="alert">{p.error}</div>{/if}
