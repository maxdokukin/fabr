<script lang="ts">
  import Row from '../ui/Row.svelte';
  import { EFFORTS, MODELS } from './wizard';

  let { bin = $bindable(), model = $bindable(), effort = $bindable(), found, onbin, onpick }: {
    bin: string;
    model: string;
    effort: string;
    found?: { path?: string; version?: string } | null;
    onbin?: () => void;
    onpick?: () => void;
  } = $props();
</script>

<Row label="Program">
  <input class="row-input" type="text" bind:value={bin} oninput={onbin} placeholder="claude (from PATH)" spellcheck="false" autocomplete="off" aria-label="Claude Code program path" />
</Row>
{#if found?.path}<Row label="Found" value={found.path} wrap />{/if}
{#if found?.version}<Row label="Version" value={found.version} />{/if}
<Row label="Model">
  <select class="row-input" bind:value={model} onchange={onpick} aria-label="Model">{#each MODELS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}</select>
</Row>
<Row label="Effort">
  <select class="row-input" bind:value={effort} onchange={onpick} aria-label="Effort">{#each EFFORTS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}</select>
</Row>
