<script lang="ts">
  import { untrack } from 'svelte';
  import type { Store } from '../store.svelte';
  import Page from '../ui/Page.svelte';
  import AdvancedPage from './AdvancedPage.svelte';
  import ClaudePage from './ClaudePage.svelte';
  import AppearancePage from './AppearancePage.svelte';
  import { SettingsController } from './controller.svelte';
  import RootPage from './RootPage.svelte';
  import ShortcutsPage from './ShortcutsPage.svelte';
  import TelegramPage from './TelegramPage.svelte';
  import SourcePage from './SourcePage.svelte';
  import SourcesPage from './SourcesPage.svelte';

  let { store }: { store: Store } = $props();
  const ctl = new SettingsController(untrack(() => store));

  let wasOpen = false;
  $effect(() => {
    const open = store.sheetOpen;
    if (open && !wasOpen) {
      ctl.reset(untrack(() => store.sheetPage));
      store.sheetPage = null;
    }
    wasOpen = open;
  });

  $effect(() => {
    void store.tree;
    untrack(() => ctl.reconcile());
  });

  let sheet = $state<HTMLDivElement>();
  $effect(() => {
    void ctl.stack.length;
    if (store.sheetOpen) sheet?.focus();
  });

  function onkeydown(e: KeyboardEvent) {
    if (!store.sheetOpen || e.key !== 'Escape') return;
    e.preventDefault();
    e.stopPropagation();
    void ctl.back();
  }
  const draft = $derived(ctl.draft);
</script>

<svelte:window {onkeydown} />

{#if store.sheetOpen}
  <div class="sheet__backdrop" role="presentation" onmousedown={(e) => { if (e.target === e.currentTarget) void ctl.done(); }}>
    <div class="sheet settings-sheet" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" bind:this={sheet}>
      <Page title={ctl.title} back={ctl.backLabel} onback={() => ctl.back()} ondone={() => ctl.done()}>
        {#key ctl.stack.length + ':' + (draft?.uid ?? ctl.page.kind)}
          {#if ctl.page.kind === 'root'}<RootPage {store} {ctl} />
          {:else if ctl.page.kind === 'appearance'}<AppearancePage {store} />
          {:else if ctl.page.kind === 'sources'}<SourcesPage {ctl} />
          {:else if ctl.page.kind === 'advanced'}<AdvancedPage {store} {ctl} />
          {:else if ctl.page.kind === 'claude'}<ClaudePage {ctl} />
          {:else if ctl.page.kind === 'telegram'}<TelegramPage {ctl} />
          {:else if ctl.page.kind === 'keys'}<ShortcutsPage />
          {:else if draft}<SourcePage {store} {ctl} d={draft} />
          {/if}
        {/key}
      </Page>
    </div>
  </div>
{/if}
