<script lang="ts">
  import Crumbs from './Crumbs.svelte';
  import { rollup, type GroupBy, type ViewMode } from './model';
  import { FILTERS, countParts, countsLabel, showLabel } from './viewstate';
  import Popover from './ui/Popover.svelte';
  import type { Store, View } from './store.svelte';
  import Button from './ui/Button.svelte';
  import Icon from './ui/Icon.svelte';
  import Segmented from './ui/Segmented.svelte';
  import AgentsBadge from './fabric/AgentsBadge.svelte';
  import NoticesBadge from './fabric/NoticesBadge.svelte';
  import type { FabricStore } from './fabric/tasks.svelte';

  let { store, onrescan, onexpand, oncollapse, onsearchenter, fabric = null, onnotices, onagents }: {
    store: Store;
    onrescan: () => void;
    onexpand: () => void;
    oncollapse: () => void;
    onsearchenter: () => void;
    fabric?: FabricStore | null;
    onnotices?: () => void;
    onagents?: () => void;
  } = $props();

  let search = $state<HTMLInputElement>();
  let menu = $state<Popover>();
  export function focusSearch() {
    search?.focus();
    search?.select();
  }

  const VIEWS: { value: View; label: string }[] = [
    { value: 'tree', label: 'Tree' },
    { value: 'list', label: 'List' },
  ];
  const MODES: { value: ViewMode; label: string; title: string }[] = [
    { value: 'nodes', label: 'Nodes', title: 'Only directories with a .fabr' },
    { value: 'folders', label: 'Folders', title: 'Plain directories as far as each node’s show_depth' },
    { value: 'all', label: 'All', title: 'Every directory' },
  ];
  const GROUPS = $derived<{ value: GroupBy; label: string }[]>([
    { value: 'none', label: 'None' },
    { value: 'node', label: 'Node' },
    { value: 'status', label: 'Status' },
    ...(store.multiSource ? [{ value: 'source' as const, label: 'Source' }] : []),
    { value: 'due', label: 'Due' },
  ]);

  const label = $derived(showLabel(store));

  // True when the agent's search root differs from the opened workspace, or there are
  // out-of-scope records / presence elsewhere. Derives from the store's own scope summary.
  const scopeMismatch = $derived.by(() => {
    if (!fabric) return false;
    const { workerRoot, openedRoots, outOfScope } = fabric.scope;
    const rootMismatch = workerRoot !== null && openedRoots.length > 0 && !openedRoots.some((r) => r === workerRoot);
    return rootMismatch || outOfScope.length > 0 || fabric.presenceElsewhere;
  });
  const backendTitle = $derived(
    fabric ? (fabric.workerRoot ? `${fabric.backendLabel} · searching ${fabric.workerRoot}` : fabric.backendLabel) : '',
  );

  const root = $derived(store.displayRoot);
  const tally = $derived(root && store.today ? rollup(store.tallyNode(root), store.today) : null);
  const counts = $derived(tally ? countParts(tally) : null);

  function act(fn: () => void) {
    menu?.close();
    fn();
  }
</script>

<header class="toolbar">
  <div class="toolbar__head">
    <Popover bind:this={menu} label="View options" width={340}>
      {#snippet trigger({ toggle, open, id })}
        <Button variant="tint" class="viewbtn" on={open || label !== 'View'} data-popover-trigger aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? id : undefined}
          title="How the tree and list look and what they show" onclick={toggle}>{label} <Icon name="chevron-down" size={10} /></Button>
      {/snippet}
      <section class="popgroup">
        <h3 class="popgroup__head">View as</h3>
        <div class="group">
          <div class="prow prow--seg">
            <Segmented size="sm" label="View as" options={VIEWS} bind:value={store.view} onchange={() => store.savePrefs()} />
          </div>
          {#if store.view === 'list'}
            <div class="prow">
              <span class="prow__label">Group by</span>
              <Segmented size="sm" label="Group by" options={GROUPS} bind:value={store.list.groupBy} onchange={() => store.savePrefs()} />
            </div>
          {/if}
        </div>
      </section>
      <section class="popgroup">
        <h3 class="popgroup__head">See</h3>
        <div class="group">
          <div class="prow prow--seg">
            <Segmented size="sm" label="Filter items" options={FILTERS} bind:value={store.filter} onchange={() => store.savePrefs()} />
          </div>
        </div>
      </section>
      <section class="popgroup">
        <h3 class="popgroup__head">Folders</h3>
        <div class="group">
          <div class="prow">
            <span class="prow__label">Show</span>
            <Segmented size="sm" label="Directories shown" options={MODES} bind:value={store.viewMode} onchange={(m) => store.setViewMode(m)} />
          </div>
          <label class="prow">
            <span class="prow__label">Hidden nodes</span>
            <input class="switch" type="checkbox" role="switch" checked={store.showHidden} onchange={(e) => store.setShowHidden(e.currentTarget.checked)} />
          </label>
          <label class="prow">
            <span class="prow__label">Done items</span>
            <input class="switch" type="checkbox" role="switch" checked={!store.hideDone} onchange={() => store.setHideDone(!store.hideDone)} />
          </label>
          <label class="prow prow--sub" title="Show or hide empty nodes (e)">
            <span class="prow__label">Empty nodes<span class="prow__sub">hide nodes with no task matching See</span></span>
            <input class="switch" type="checkbox" role="switch" checked={!store.hideEmpty} onchange={() => store.setHideEmpty(!store.hideEmpty)} />
          </label>
        </div>
      </section>
      {#if store.view === 'tree'}
        <section class="popgroup">
          <h3 class="popgroup__head">Tree</h3>
          <div class="group">
            <button type="button" class="arow" title="Expand every node" onclick={() => act(onexpand)}>Expand all</button>
            <button type="button" class="arow" title="Collapse below the top level" onclick={() => act(oncollapse)}>Collapse all</button>
          </div>
        </section>
      {/if}
    </Popover>
    {#if counts}
      <p class="counts" aria-label={tally ? countsLabel(tally) : undefined}>
        {#each counts as c, i (c.label)}
          {#if i}<span class="dotsep">·</span>{/if}<span class="counts__part counts__part--{c.tone}"><span class="num">{c.n}</span> {c.label}</span>
        {/each}
      </p>
    {/if}
    <span class="toolbar__spacer"></span>
    <label class="search">
      <Icon name="search" size={14} class="search__icon" />
      <input
        bind:this={search}
        type="search"
        placeholder="Search"
        aria-label="Search nodes and items"
        bind:value={store.query}
        onkeydown={(e) => { if (e.key === 'Enter') onsearchenter(); }}
      />
      <kbd class="search__kbd" aria-hidden="true">/</kbd>
    </label>
    {#if fabric}
      <span class="backend backend--{fabric.backend}" class:backend--warn={scopeMismatch} title={backendTitle} aria-label={backendTitle} role="status"><span class="backend__dot"></span></span>
      <AgentsBadge agents={fabric.agentList} onclick={onagents} />
      <NoticesBadge events={fabric.events} readPosition={fabric.readPosition} onclick={onnotices} />
    {/if}
    <Button variant="icon" title="Rescan (r)" aria-label="Rescan" onclick={onrescan}><Icon name="refresh" size={15} /></Button>
    <Button variant="icon" title="Settings" aria-label="Settings" onclick={() => (store.sheetOpen = true)}><Icon name="gear" size={17} /></Button>
  </div>
  <div class="toolbar__sub">
    <Crumbs {store} />
  </div>
</header>

<style>
  .backend { display: inline-flex; align-items: center; padding: 0 4px; }
  .backend__dot { width: 8px; height: 8px; border-radius: 50%; background: var(--label-3); }
  .backend--connected .backend__dot { background: var(--green); }
  .backend--stale .backend__dot { background: var(--orange); }
  .backend--warn .backend__dot { background: var(--orange); box-shadow: 0 0 0 2px color-mix(in srgb, var(--orange) 35%, transparent); }
</style>
