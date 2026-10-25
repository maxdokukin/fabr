<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { createApi } from './lib/api/index';
  import { needsOnboarding } from './lib/addnodes';
  import AddNodesSheet from './lib/settings/AddNodesSheet.svelte';
  import Onboarding from './lib/Onboarding.svelte';
  import ListView from './lib/list/ListView.svelte';
  import Notice from './lib/Notice.svelte';
  import SettingsSheet from './lib/settings/SettingsSheet.svelte';
  import NodeSettingsSheet from './lib/settings/NodeSettingsSheet.svelte';
  import TaskSheet from './lib/fabric/TaskSheet.svelte';
  import { Store } from './lib/store.svelte';
  import SetupWizard from './lib/setup/SetupWizard.svelte';
  import { shouldShowSetup } from './lib/setup/gate';
  import AgentsScreen from './lib/fabric/AgentsScreen.svelte';
  import NoticesList from './lib/fabric/NoticesList.svelte';
  import { FabricStore, localRoots } from './lib/fabric/tasks.svelte';
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
  let agentsOpen = $state(false);
  let noticeError = $state<string | null>(null);
  let setupOpen = $state(new URLSearchParams(location.search).has('setup'));
  let gateReady = $state(false);

  let gateApi: Awaited<ReturnType<typeof createApi>> | null = null;
  let setupNote = $state<string | null>(null);
  let alive = true;
  let agentReason = $state<string | null>(null);

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
    if (!setupOpen) {
      startStore();
      gateApi.fabricAgentStatus().then((s) => (agentReason = s.reason), () => {});
    }
  }

  async function setupDone() {
    let read;
    try { read = await gateApi!.fabricSettingsRead(); }
    catch (e) { setupNote = `Could not confirm your settings: ${(e as Error).message}`; return; }
    if (shouldShowSetup(read)) { setupNote = 'Setup is not finished yet. Complete the remaining steps to continue.'; return; }
    setupNote = null;
    setupOpen = false;
    startStore();
    gateApi!.fabricAgentStart().then((s) => (agentReason = s.reason), (e) => (agentReason = (e as Error).message));
  }

  onMount(() => {
    createApi().then(checkGate);
    return () => { alive = false; store?.stop(); fabric?.stop(); };
  });

  $effect(() => {
    if (store?.tree && !fabric) {
      fabric = new FabricStore();
      fabric.onItemsChange = () => store?.reloadSoon();
      void fabric.start(localRoots(store.tree.sources), store.tree.sources);
    }
  });

  $effect(() => { if (fabric && store?.tree) fabric.setSources(store.tree.sources); });

  $effect(() => { if (fabric) fabric.agentReason = agentReason; });

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
    noticeError = null;
    noticesOpen = false;
    agentsOpen = false;
    // TaskSheet mounts the open record from fabric.openPath regardless of whether a tree node
    // hosts its folder, so opening the resolved record is never a dead-end (Bug B, 2.1/2.4).
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
    if (!store || store.sheetOpen || store.nodeSettings || store.addNodesFor || helpOpen || noticesOpen || agentsOpen || fabric?.openPath) return;
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
    }
  }
</script>

<svelte:window {onkeydown} />

{#if gateReady && setupOpen}
  {#if setupNote}<p class="gate-note" role="alert">{setupNote}</p>{/if}
  <SetupWizard ondone={setupDone} />
{:else if store}
  <div class="app">
    <main class="app__main">
      <div class="app__bar" bind:clientHeight={toolbarH}>
        <Toolbar bind:this={toolbar} {store} onrescan={rescan} onexpand={expandAll} oncollapse={collapseAll} onsearchenter={() => treeView?.nextMatch()} {fabric} onnotices={() => { noticeError = null; noticesOpen = true; }} onagents={() => { noticeError = null; agentsOpen = true; }} />
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
  </div>
  <SettingsSheet {store} />
  <NodeSettingsSheet {store} />
  <AddNodesSheet {store} />
  {#if fabric}
    <TaskSheet {fabric} />
    {@const nf = fabric}
    <Sheet open={noticesOpen} title="Notices" onclose={() => (noticesOpen = false)}>
      <NoticesList events={fabric.events} readPosition={fabric.readPosition} titleFor={fabric.titleFor} outOfScope={(p: string) => { const h = nf.locate(p); return !!h && nf.scope.outOfScope.includes(h); }} onopen={openNotice} onmarkallread={() => fabric?.markAllRead()} />
      {#if noticeError}<p class="gate-note" role="alert">{noticeError}</p>{/if}
      {#snippet footer()}<Button variant="tint" onclick={() => (noticesOpen = false)}>Close</Button>{/snippet}
    </Sheet>
    <Sheet open={agentsOpen} title="Agents" onclose={() => (agentsOpen = false)}>
      <AgentsScreen agents={fabric.agentList} now={fabric.now} onopen={openNotice} />
      {#if noticeError}<p class="gate-note" role="alert">{noticeError}</p>{/if}
      {#snippet footer()}<Button variant="tint" onclick={() => (agentsOpen = false)}>Close</Button>{/snippet}
    </Sheet>
  {/if}
  <Shortcuts open={helpOpen} onclose={() => (helpOpen = false)} />
{/if}

<style>
  .gate-note { margin: 0; padding: 12px 16px; color: var(--red); font-size: var(--fs-13, 13px); text-align: center; }
</style>
