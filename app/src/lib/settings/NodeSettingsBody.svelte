<script lang="ts">
  import { isVirtualRoot, rootCounts, sourceLines } from '../model';
  import { KNOWN_META, removeSummary, type NodeSettingsState, type Store } from '../store.svelte';
  import ErrorText from '../ui/ErrorText.svelte';
  import Group from '../ui/Group.svelte';
  import Icon from '../ui/Icon.svelte';
  import Row from '../ui/Row.svelte';
  import BannerView from '../panel/Banner.svelte';
  import NodeClaude from './NodeClaude.svelte';
  import MetaEditor from '../panel/MetaEditor.svelte';

  let { store, p }: { store: Store; p: NodeSettingsState } = $props();

  let busy = $state(false);
  let confirmRemove = $state(false);

  const src = $derived(store.sourceOf(p.source));
  const virtual = $derived(isVirtualRoot(p));
  const canReveal = $derived(!virtual && !src?.host && !store.revealUnsupported);
  const shownRoot = $derived(virtual ? (store.viewIndex.byKey.get(':') ?? null) : null);
  const tallyRoot = $derived(shownRoot ? store.tallyNode(shownRoot) : null);
  const counts = $derived(shownRoot ? rootCounts(tallyRoot!, store.today) : null);
  const lines = $derived(shownRoot && store.tree ? sourceLines(tallyRoot!, store.tree.sources, store.tree.errors, store.today, undefined, store.tree.pending ?? []) : []);
  const hidden = $derived(p.meta.some(([k, v]) => k === 'hidden' && (v === true || v === 'true')));
  const items = $derived(store.byKey.get(p.key)?.items ?? []);
  const summary = $derived(removeSummary({ items }));
  const isSourceRoot = $derived(p.path === '');

  const metaProps = $derived({
    entries: p.meta,
    known: KNOWN_META,
    note: p.metaNote,
    inherited: store.inheritedColor(p.key),
    oncommit: (k: string, v: string | number | boolean | undefined) => store.commitMeta(k, v),
    onerror: (m: string | null) => { p.banner = m ? { kind: 'error' as const, text: m } : null; },
  });

  $effect(() => {
    void p.key;
    confirmRemove = false;
  });

  async function run(fn: () => Promise<void>) {
    busy = true;
    try {
      await fn();
    } finally {
      busy = false;
    }
  }
</script>

<BannerView banner={p.banner} />
{#if p.warnings.length}
  <ul class="nodeset__warnings">
    {#each p.warnings as w, i (i)}<li><Icon name="warning" size={12} /> {w}</li>{/each}
  </ul>
{/if}

{#if virtual}
  {#if counts}
    <section class="nodeset__section">
      <h3 class="section-title">Tasks</h3>
      <p class="rootcounts">
        <span><b>{counts.todo}</b> todo</span><span class="dotsep">·</span>
        <span class:card__wip={counts.active > 0}><b>{counts.active}</b> active</span><span class="dotsep">·</span>
        <span class:card__overdue={counts.overdue > 0}><b>{counts.overdue}</b> overdue</span>
      </p>
    </section>
  {/if}
  <section class="nodeset__section">
    <h3 class="section-title">Sources <span class="section-title__count">{lines.length}</span></h3>
    <div class="group">
      {#each lines as l (l.key)}
        <button type="button" class="arow arow--source" title={l.error ?? `Open ${l.name}`} onclick={() => store.openNodeSettings(l.key)}>
          <span class="dot" class:dot--ok={!l.error && !l.pending} class:dot--wait={!l.error && l.pending} class:dot--bad={!!l.error}></span>
          <span class="arow__label">{l.name} · {l.where}</span>
          {#if l.error}<span class="srcline__error"><ErrorText message={l.error} compact onopen={(u) => store.openUrl(u)} /></span>{:else if l.pending}<span class="srcline__error srcline__pending">Connecting…</span>{:else}<span class="srcline__open">{l.open} open</span>{/if}
        </button>
      {/each}
      <button type="button" class="arow" onclick={() => store.openSettings('sources')}>Manage sources…</button>
    </div>
    <p class="hint">Every configured source, side by side. Add, edit or test sources in Settings.</p>
  </section>
{:else if !p.managed}
  <div class="empty-node">
    <Icon name="folder" size={28} />
    <p class="empty-node__title">Plain directory</p>
    <p class="hint">It has no <code>.fabr/</code> folder yet. Make it a node to give it tasks and metadata.</p>
  </div>
  <section class="nodeset__section">
    <h3 class="section-title">Actions</h3>
    <Group>
      <Row label="Make it a node" tone="tint" disabled={busy} onclick={() => run(() => store.init(p.key))} />
      <Row label="Add nodes…" tone="tint" onclick={() => store.openAddNodes(p.key)} />
      {#if canReveal}<Row label="Reveal in Finder" tone="tint" onclick={() => store.reveal()} />{/if}
    </Group>
  </section>
{:else}
  <section class="nodeset__section">
    <Group footer="Hidden omits this node and its whole subtree (see Show in the toolbar).">
      <Row label={hidden ? 'Unhide' : 'Hide'} tone="tint" disabled={busy} onclick={() => run(async () => { await store.setHidden(!hidden); })} />
    </Group>
  </section>

  <section class="nodeset__section">
    <h3 class="section-title">Identity</h3>
    <MetaEditor section="identity" {...metaProps} />
  </section>

  <section class="nodeset__section">
    <h3 class="section-title">Display</h3>
    <MetaEditor section="display" {...metaProps} />
  </section>

  <section class="nodeset__section">
    <h3 class="section-title">Custom keys</h3>
    <MetaEditor section="more" {...metaProps} />
    <p class="hint">Fields save on Enter or when you leave them.</p>
  </section>

  {#if src && !src.host}<NodeClaude root={src.root} path={p.path} onadded={() => store.reloadSoon()} />{/if}

  <section class="nodeset__section">
    <h3 class="section-title">Actions</h3>
    <Group>
      <Row label="Add child nodes…" tone="tint" onclick={() => store.openAddNodes(p.key)} />
      {#if canReveal}<Row label="Reveal in Finder" tone="tint" onclick={() => store.reveal()} />{/if}
    </Group>
    {#if confirmRemove}
      <div class="confirm" role="alertdialog" aria-label="Remove node">
        <p class="confirm__text">
          Removes <code>.fabr/</code> from {p.path || src?.root || p.name}. The folder and its files stay; {summary.text}.
          Subfolder nodes keep theirs.{#if isSourceRoot} The source will show as a plain folder.{/if}
        </p>
        <Group>
          <Row label="Remove" tone="danger" disabled={busy} onclick={() => run(async () => { await store.removeNode(); })} />
          <Row label="Cancel" tone="tint" disabled={busy} onclick={() => (confirmRemove = false)} />
        </Group>
      </div>
    {:else}
      <Group>
        <Row label="Remove node…" tone="danger" disabled={busy} onclick={() => (confirmRemove = true)} />
      </Group>
    {/if}
  </section>
{/if}
