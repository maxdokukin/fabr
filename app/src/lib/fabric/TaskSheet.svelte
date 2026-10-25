<script lang="ts">
  import Sheet from '../ui/Sheet.svelte';
  import AgentsScreen from './AgentsScreen.svelte';
  import StageBar from './StageBar.svelte';
  import { finishedView, suspendedView } from './stage';
  import { folderName, folderOf } from './folder';
  import RecordScreen from './RecordScreen.svelte';
  import type { FabricStore } from './tasks.svelte';

  let { fabric, folder = null }: { fabric: FabricStore; folder?: string | null } = $props();

  const open = $derived(fabric.openRecord && (folder === null || folderOf(fabric.openRecord.path) === folder) ? fabric.openRecord : null);
  const ro = $derived(fabric.openReadonly);
</script>

{#if open}
  {@const h = open}
  <Sheet open={true} title={folderName(folderOf(h.path))} width={720} onclose={() => fabric.open(null)}>
    <RecordScreen
      record={h.record}
      path={h.path}
      name={folderName(folderOf(h.path))}
      backLabel="Close"
      now={fabric.now}
      onback={() => fabric.open(null)}
      onanswer={(a) => fabric.answer(h, a)}
      onapprove={(d) => fabric.approve(h, d)}
      onrequest={(k) => (k === 'pause' ? fabric.pause(h.path) : k === 'resume' ? fabric.resume(h.path) : fabric.request(h, k))}
      onedit={(s, text) => (s === 'assertions' ? fabric.editAssertions(h.path, text) : fabric.editSection(h.path, s, text))}
      onfields={(f) => fabric.editFields(h.path, f)}
      onsuspend={(until) => fabric.suspend(h.path, until)}
      onunsuspend={() => fabric.unsuspend(h.path)}
      onreload={() => fabric.reload(h.path)}
      readonly={ro}
    >
      {#snippet agent()}
        {@const view = fabric.agents.get(h.path)}
        {@const done = finishedView(h)}
        {@const shelf = suspendedView(h, fabric.backend, fabric.now)}
        {#if shelf}
          <StageBar view={shelf} mode="full" now={fabric.now} />
        {:else if view}
          <AgentsScreen agents={[view]} detail now={fabric.now} oncancel={(p) => fabric.cancelTask(p)} />
        {:else if done}
          <StageBar view={done} mode="full" now={fabric.now} />
        {/if}
      {/snippet}
    </RecordScreen>
  </Sheet>
{/if}
