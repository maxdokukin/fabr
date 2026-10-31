<script lang="ts">
  import type { Transcript } from '../api/types';
  import { toolLine } from './cc';
  import Prose from './Prose.svelte';

  let { transcript }: { transcript: Transcript } = $props();
</script>

{#if transcript.truncated}<p class="hint">Showing the last {transcript.entries.length} entries</p>{/if}
<div class="tr" data-testid="transcript">
  {#each transcript.entries as e, i (i)}
    {#if e.role === 'tool'}
      <p class="tool" class:tool--bad={e.tool === 'error'}>{toolLine(e)}</p>
    {:else}
      {#if e.role === 'assistant'}<div class="msg msg--assistant"><Prose text={e.text} /></div>{:else}<p class="msg msg--{e.role}">{e.text}</p>{/if}
    {/if}
  {:else}
    <p class="hint">Nothing in this session yet.</p>
  {/each}
</div>

<style>
  .tr { display: flex; flex-direction: column; gap: 8px; }
  p { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--fs-13, 13px); line-height: 1.45; }
  .msg--user { background: var(--bg-elev); border: 0.5px solid var(--sep); border-radius: 10px; padding: 8px 12px; color: var(--label); max-height: 14em; overflow: auto; }
  .msg--assistant { padding: 0 4px; color: var(--label); font-size: var(--fs-13, 13px); line-height: 1.45; }
  .tool { font: 11.5px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--label-2); padding: 0 4px; }
  .tool--bad { color: var(--red); }
  .hint { margin-bottom: 8px; }
</style>
