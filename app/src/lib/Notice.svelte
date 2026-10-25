<script lang="ts">
  import type { Store } from './store.svelte';
  import type { FabricStore } from './fabric/tasks.svelte';
  import ErrorText from './ui/ErrorText.svelte';
  import Icon from './ui/Icon.svelte';
  let { store, fabric = null, insetTop = 0 }: { store: Store; fabric?: FabricStore | null; insetTop?: number } = $props();
  const errors = $derived(store.tree?.errors ?? []);
  let dismissed = $state<string>('');
  const sig = $derived(errors.map((e) => `${e.source}:${e.message}`).join('|'));

  // True when the agent's search root differs from the opened workspace, or there are
  // out-of-scope records / presence elsewhere. Mirrors the toolbar's scope summary (task 7.1).
  const scopeMismatch = $derived.by(() => {
    if (!fabric) return false;
    const { workerRoot, openedRoots, outOfScope } = fabric.scope;
    const rootMismatch = workerRoot !== null && openedRoots.length > 0 && !openedRoots.some((r) => r === workerRoot);
    return rootMismatch || outOfScope.length > 0 || fabric.presenceElsewhere;
  });
  let scopeDismissed = $state(false);
</script>

<div class="notices" style:top="{insetTop + 10}px" aria-live="polite">
  {#if store.notice}
    <div class="notice" role="status">
      <Icon name="warning" size={14} class="notice__icon" />
      <span class="notice__text">{store.notice}</span>
      {#if store.noticeRescan}<button type="button" class="btn btn--tint notice__action" onclick={() => store.rescan()}>Rescan</button>{/if}
      <button type="button" class="btn btn--icon" aria-label="Dismiss" onclick={() => (store.notice = null)}><Icon name="x" size={12} /></button>
    </div>
  {/if}
  {#if scopeMismatch && fabric?.workerRoot && !scopeDismissed}
    <div class="notice" role="status" data-testid="scope-notice">
      <Icon name="warning" size={14} class="notice__icon" />
      <span class="notice__text">Agent is searching {fabric.workerRoot}; the opened workspace differs. Scope default_folder to this project.</span>
      <button type="button" class="btn btn--icon" aria-label="Dismiss" onclick={() => (scopeDismissed = true)}><Icon name="x" size={12} /></button>
    </div>
  {/if}
  {#if errors.length && dismissed !== sig}
    <div class="notice notice--error" role="alert">
      <Icon name="warning" size={14} class="notice__icon" />
      <span class="notice__text">
        {#each errors as e, i (i)}<span class="notice__line"><b>{store.sourceOf(e.source)?.name ?? e.source}</b> <ErrorText message={e.message} onopen={(u) => store.openUrl(u)} /></span>{/each}
      </span>
      <button type="button" class="btn btn--icon" aria-label="Dismiss" onclick={() => (dismissed = sig)}><Icon name="x" size={12} /></button>
    </div>
  {/if}
</div>
