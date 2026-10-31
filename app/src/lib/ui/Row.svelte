<script lang="ts">
  import type { Snippet } from 'svelte';
  let { label, sub, value, tone, wrap = false, onclick, disabled = false, lead, children }: {
    label?: string;
    sub?: string;
    value?: string;
    tone?: 'danger' | 'tint' | 'error';
    wrap?: boolean;
    onclick?: () => void;
    disabled?: boolean;
    lead?: Snippet;
    children?: Snippet;
  } = $props();
</script>

{#if onclick}
  <button type="button" class="row-item row-item--button" class:row-item--wrap={wrap} data-tone={tone} {disabled} {onclick}>
    {#if lead}{@render lead()}{/if}
    {#if label}<span class="row-label"><span class="row-main">{label}</span>{#if sub}<span class="row-sub">{sub}</span>{/if}</span>{/if}
    {#if value}<span class="row-value">{value}</span>{/if}
    {#if children}{@render children()}{/if}
  </button>
{:else}
  <label class="row-item" class:row-item--wrap={wrap} data-tone={tone}>
    {#if lead}{@render lead()}{/if}
    {#if label}<span class="row-label"><span class="row-main">{label}</span>{#if sub}<span class="row-sub">{sub}</span>{/if}</span>{/if}
    {#if value}<span class="row-value">{value}</span>{/if}
    {#if children}{@render children()}{/if}
  </label>
{/if}
