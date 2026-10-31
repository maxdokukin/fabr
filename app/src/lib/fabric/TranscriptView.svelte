<script lang="ts">
  import type { Transcript } from '../api/types';
  import { toolLine } from './cc';

  let { transcript }: { transcript: Transcript } = $props();
</script>

{#if transcript.truncated}<p class="hint">Showing the last {transcript.entries.length} entries</p>{/if}
<div class="tr" data-testid="transcript">
  {#each transcript.entries as e, i (i)}
    {#if e.role === 'tool'}
      <p class="tool" class:tool--bad={e.tool === 'error'}>{toolLine(e)}</p>
    {:else}
      <p class="msg msg--{e.role}">{e.text}</p>
    {/if}
  {:else}
    <p class="hint">Nothing in this session yet.</p>
  {/each}
</div>

<style>
  .tr { display: flex; flex-direction: column; gap: 8px; }
  p { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .msg--user { font-weight: 600; }
  .tool { font: 12px ui-monospace, monospace; opacity: 0.6; }
  .tool--bad { color: var(--danger, #d33); opacity: 1; }
</style>
