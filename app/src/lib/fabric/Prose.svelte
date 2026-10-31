<script lang="ts">
  import { proseBlocks } from './prose';
  let { text }: { text: string } = $props();
  const blocks = $derived(proseBlocks(text));
</script>

<div class="prose">
  {#each blocks as b, i (i)}
    {#if b.kind === 'gap'}<div class="gap"></div>
    {:else}<div class={b.kind}>{#each b.segs as s, j (j)}{#if s.code}<code>{s.text}</code>{:else if s.bold}<b>{s.text}</b>{:else}{s.text}{/if}{/each}</div>{/if}
  {/each}
</div>

<style>
  .prose { overflow-wrap: anywhere; }
  .gap { height: 6px; }
  .h { font-weight: 600; margin-top: 2px; }
  .li { padding-left: 4px; }
  code { font-family: var(--font-mono, ui-monospace, monospace); font-size: 0.92em; background: var(--fill-3, rgba(127, 127, 127, 0.14)); border-radius: 4px; padding: 0 3px; }
</style>
