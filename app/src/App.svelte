<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { createApi } from './lib/api/index';
  import { nodeKey } from './lib/model';
  import { needsOnboarding } from './lib/addnodes';
  import AddNodesSheet from './lib/settings/AddNodesSheet.svelte';
  import Onboarding from './lib/Onboarding.svelte';
  import ListView from './lib/list/ListView.svelte';
  import Notice from './lib/Notice.svelte';
  import Panel from './lib/panel/Panel.svelte';
  import SettingsSheet from './lib/settings/SettingsSheet.svelte';
  import { Store } from './lib/store.svelte';
  import SetupWizard from './lib/setup/SetupWizard.svelte';
  import { shouldShowSetup } from './lib/setup/gate';
  import NoticesList from './lib/fabric/NoticesList.svelte';
  import RecordScreen from './lib/fabric/RecordScreen.svelte';
  import AgentsScreen from './lib/fabric/AgentsScreen.svelte';
  import { FabricStore, localRoots } from './lib/fabric/tasks.svelte';
  import { folderName, folderOf } from './lib/fabric/folder';
  import Button from './lib/ui/Button.svelte';
  import Sheet from './lib/ui/Sheet.svelte';
  import Toolbar from './lib/Toolbar.svelte';
  import Shortcuts from './lib/ui/Shortcuts.svelte';
  import { keyAction } from './lib/viewstate';
  import TreeView from './lib/tree/TreeView.svelte';

  let store = $state<Store | null>(null);
  let treeView = $state<TreeView>();
  let toolbar = $state<Toolbar>();
  let toolbarH = $state(0);
  let helpOpen = $state(false);
  let fabric = $state<FabricStore | null>(null);
  let noticesOpen = $state(false);
  let noticeError = $state<string | null>(null);
  let setupOpen = $state(new URLSearchParams(location.search).has('setup'));
  let gateReady = $state(false);

  let gateApi: Awaited<ReturnType<typeof createApi>> | null = null;
  let setupNote = $state<string | null>(null);
  let alive = true;

  function startStore() {
    if (store || !gateApi || !alive) return;
    store = new Store(gateApi);
    store.start();
  }

  async function checkGate(api: Awaited<ReturnType<typeof createApi>>) {
    gateApi = api;
    try {
      if (shouldShowSetup(await api.fabricSettingsRead())) setupOpen = true;
    } catch {
      setupOpen = true;
      setupNote = 'Could not read your settings. Finish setup to continue.';
    }
    gateReady = true;
    if (!setupOpen) startStore();
  }

  async function setupDone() {
    let read;
    try { read = await gateApi!.fabricSettingsRead(); }
    catch (e) { setupNote = `Could not confirm your settings: ${(e as Error).message}`; return; }
    if (shouldShowSetup(read)) { setupNote = 'Setup is not finished yet. Complete the remaining steps to continue.'; return; }
    setupNote = null;
    setupOpen = false;
    startStore();
  }

  onMount(() => {
    createApi().then(checkGate);
    return () => { alive = false; store?.stop(); fabric?.stop(); };
  });

  $effect(() => {
    if (store?.tree && !fabric) {
      fabric = new FabricStore();
      void fabric.start(localRoots(store.tree.sources));
    }
  });

  // The open record only when NO visible tree node hosts its folder. In that case TaskSection's
  // folderOf(openRecord.path) === folder gate never matches, so the record would never mount; the
  // standalone sheet below renders it instead. When a node does host the folder, TaskSection shows
  // the in-tree sheet and this stays null to avoid a double sheet (preserve 3.3).
  let standaloneRecord = $derived.by(() => {
    const open = fabric?.openRecord;
    if (!open) return null;
    const folder = folderOf(open.path);
    const hosted = store && [...store.byKey.values()].some((n) => fabric!.folderFor(store!.sourceOf(n.source), n.path) === folder);
    return hosted ? null : open;
  });

  function openNotice(taskPath: string) {
    // `locate` is redelegation/path-form aware (view.ts findByTaskPath): `held` is the live target.
    const held = fabric?.locate(taskPath);
    if (!held) {
      // Distinguish a record that is on disk but failed to parse (recoverable: fix the file)
      // from one that is genuinely missing — both are recoverable, never a silent dead-end.
      const reason = fabric?.unreadableReason(taskPath);
      noticeError = reason
        ? `This record is on disk but could not be read (${reason}). Fix the file, then rescan.`
        : `No record on disk for ${taskPath}. Try rescanning or check the folder.`;
      return;
    }
    const folder = folderOf(held.path);
    const node = store ? [...store.byKey.values()].find((n) => fabric!.folderFor(store!.sourceOf(n.source), n.path) === folder) : undefined;
    noticeError = null;
    noticesOpen = false;
    // In-tree: open the hosting node's panel exactly as before (preserve 3.3). Out-of-tree: no
    // node hosts this folder, so the standalone record sheet mounts from fabric.openPath (task 6.2).
    if (node) store!.openPanel(nodeKey(node));
    fabric!.open(held.path);
  }

  function fit() {
    if (store?.view === 'tree') treeView?.fit();
  }
  function rescan() {
    store?.rescan();
  }
  async function collapseAll() {
    store?.collapseAll();
    await tick();
    fit();
  }
  async function expandAll() {
    store?.expandAll();
    await tick();
    fit();
  }

  function onkeydown(e: KeyboardEvent) {
    if (!store || store.sheetOpen || store.addNodesFor || helpOpen || noticesOpen || fabric?.openPath) return;
    const t = e.target as HTMLElement;
    const typing = !!t.matches?.('input, select, textarea, [contenteditable]');
    if (typing) {
      if (e.key === 'Escape') t.blur();
      return;
    }
    const a = keyAction(e, false);
    if (!a) return;
    switch (a.type) {
      case 'filter': store.filter = a.value; store.savePrefs(); break;
      case 'view': store.view = a.value; store.savePrefs(); break;
      case 'toggleHidden': store.setShowHidden(!store.showHidden); break;
      case 'toggleDone': store.setHideDone(!store.hideDone); break;
      case 'toggleEmpty': store.setHideEmpty(!store.hideEmpty); break;
      case 'rescan': rescan(); break;
      case 'fit': fit(); break;
      case 'search': e.preventDefault(); toolbar?.focusSearch(); break;
      case 'help': e.preventDefault(); helpOpen = true; break;
      case 'escape': if (store.panel) store.closePanel(); break;
    }
  }
</script>

<svelte:window {onkeydown} />

{#if gateReady && setupOpen}
  {#if setupNote}<p class="gate-note" role="alert">{setupNote}</p>{/if}
  <SetupWizard ondone={setupDone} />
{:else if store}
  <div class="app" class:app--panel={!!store.panel}>
    <main class="app__main">
      <div class="app__bar" bind:clientHeight={toolbarH}>
        <Toolbar bind:this={toolbar} {store} onrescan={rescan} onexpand={expandAll} oncollapse={collapseAll} onsearchenter={() => treeView?.nextMatch()} {fabric} onnotices={() => { noticeError = null; noticesOpen = true; }} />
      </div>
      <Notice {store} {fabric} insetTop={toolbarH} />
      {#if needsOnboarding(store.tree)}
        <Onboarding {store} />
      {:else if store.view === 'tree'}
        <TreeView bind:this={treeView} {store} insetTop={toolbarH} {fabric} />
      {:else}
        <ListView {store} insetTop={toolbarH} />
      {/if}
      {#if !store.tree && !store.loading && store.notice}
        <div class="empty-state empty-state--center">
          <p class="empty-state__title">Nothing to show</p>
          <p class="empty-state__text">{store.notice}</p>
        </div>
      {/if}
    </main>
    <Panel {store} {fabric} />
  </div>
  <SettingsSheet {store} />
  <AddNodesSheet {store} />
  {#if fabric}
    {@const nf = fabric}
    <Sheet open={noticesOpen} title="Notices" onclose={() => (noticesOpen = false)}>
      <NoticesList events={fabric.events} readPosition={fabric.readPosition} titleFor={fabric.titleFor} outOfScope={(p: string) => { const h = nf.locate(p); return !!h && nf.scope.outOfScope.includes(h); }} onopen={openNotice} onmarkallread={() => fabric?.markAllRead()} />
      {#if noticeError}<p class="gate-note" role="alert">{noticeError}</p>{/if}
      {#snippet footer()}<Button variant="tint" onclick={() => (noticesOpen = false)}>Close</Button>{/snippet}
    </Sheet>
    {#if standaloneRecord}
      {@const f = fabric}
      {@const h = standaloneRecord}
      <Sheet open={true} title={h.record.title} width={720} onclose={() => f.open(null)}>
        <RecordScreen
          record={h.record}
          name={folderName(folderOf(h.path))}
          backLabel="Close"
          now={f.now}
          onback={() => f.open(null)}
          onanswer={(a) => f.answer(h, a)}
          onapprove={(d) => f.approve(h, d)}
          onrequest={(k) => f.request(h, k)}
          onreload={() => f.reload(h.path)}
        >
          {#snippet agent()}
            {#if f.entryFor(h.path)}
              <AgentsScreen entries={[f.entryFor(h.path)!]} now={f.now} titleFor={f.titleFor} oncancel={(p) => f.cancelTask(p)} />
            {/if}
          {/snippet}
        </RecordScreen>
      </Sheet>
    {/if}
  {/if}
  <Shortcuts open={helpOpen} onclose={() => (helpOpen = false)} />
{/if}

<style>
  .gate-note { margin: 0; padding: 12px 16px; color: var(--red); font-size: var(--fs-13, 13px); text-align: center; }
</style>
