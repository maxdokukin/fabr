<script lang="ts">
  import { tick } from 'svelte';
  import { createApi } from '../api/index';
  import { errText, type Transcript } from '../api/types';
  import Button from '../ui/Button.svelte';
  import ErrorText from '../ui/ErrorText.svelte';
  import Sheet from '../ui/Sheet.svelte';
  import { sessionTitle } from './cc';
  import { DISCUSS_HINT } from './sessions';
  import TranscriptView from './TranscriptView.svelte';

  let { folder, id, title = '', discuss = false, onclose }: { folder: string; id: string; title?: string; discuss?: boolean; onclose: () => void } = $props();

  let t = $state<Transcript | null>(null);
  let err = $state<string | null>(null);
  let text = $state('');
  let sending = $state<string | null>(null);
  let sendErr = $state<string | null>(null);
  let cost = $state<number | null>(null);
  let area = $state<HTMLTextAreaElement>();
  let end = $state<HTMLDivElement>();

  $effect(() => {
    const f = folder, i = id;
    t = null;
    err = null;
    createApi().then((a) => a.fabricCcTranscript(f, i)).then((r) => { t = r; toEnd(); }, (e) => { err = errText(e); });
  });

  $effect(() => {
    if (discuss && area) setTimeout(() => area?.focus(), 0);
  });

  const toEnd = async () => {
    await tick();
    end?.scrollIntoView({ block: 'end' });
  };

  async function send() {
    const message = text.trim();
    if (!message || sending !== null) return;
    sending = message;
    sendErr = null;
    cost = null;
    toEnd();
    try {
      const api = await createApi();
      const r = await api.fabricCcSend({ folder, id, message });
      cost = r.cost_usd;
      text = '';
      t = await api.fabricCcTranscript(folder, id);
      err = null;
    } catch (e) {
      sendErr = errText(e);
    } finally {
      sending = null;
      toEnd();
      setTimeout(() => area?.focus(), 0);
    }
  }

  const onkeydown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
    }
  };
  const openUrl = (u: string) => { createApi().then((a) => a.openUrl(u)).catch(() => {}); };
</script>

<Sheet open={true} title="Transcript" width={720} {onclose}>
  <p class="hint tsub">{sessionTitle(t ?? { title })}</p>
  {#if err}<p class="hint">{err}</p>{:else if t}<TranscriptView transcript={t} />{:else}<p class="hint">Loading…</p>{/if}
  {#if sending !== null}
    <p class="pending" data-testid="send-pending">{sending}</p>
    <p class="hint" role="status">Claude Code is thinking…</p>
  {/if}
  <div class="composer" data-testid="composer">
    {#if discuss}<p class="hint" data-testid="discuss-hint">{DISCUSS_HINT}</p>{/if}
    <textarea class="in" rows="3" aria-label="Message" placeholder="Ask this session… (Ctrl+Enter to send)" bind:value={text} bind:this={area} disabled={sending !== null} {onkeydown}></textarea>
    <div class="crow">
      <p class="hint note">Read-only: Claude Code can look at files here but not change them.</p>
      {#if cost !== null && sending === null}<span class="hint cost" data-testid="send-cost">${cost.toFixed(2)}</span>{/if}
      <Button variant="primary" data-testid="send" disabled={!text.trim() || sending !== null} onclick={send}>Send</Button>
    </div>
    {#if sendErr}<p class="err" role="alert"><ErrorText message={sendErr} onopen={openUrl} /></p>{/if}
  </div>
  <div bind:this={end}></div>
  {#snippet footer()}<button type="button" class="btn btn--tint" onclick={onclose}>Done</button>{/snippet}
</Sheet>

<style>
  .tsub { margin: 0 4px 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .composer { display: flex; flex-direction: column; gap: 6px; margin-top: 14px; }
  .composer p { margin: 0; }
  .in { width: 100%; box-sizing: border-box; font: inherit; font-size: var(--fs-13, 13px); padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; resize: vertical; min-height: 4.5em; max-height: 9em; }
  .crow { display: flex; align-items: center; gap: 10px; }
  .note { flex: 1; }
  .cost { white-space: nowrap; }
  .pending { margin: 8px 0 0; background: var(--bg-elev); border: 0.5px solid var(--sep); border-radius: 10px; padding: 8px 12px; font-size: var(--fs-13, 13px); white-space: pre-wrap; overflow-wrap: anywhere; opacity: 0.8; }
  .err { color: var(--red); font-size: var(--fs-13, 13px); overflow-wrap: anywhere; }
</style>
