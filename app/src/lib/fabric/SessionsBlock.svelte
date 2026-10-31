<script lang="ts">
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';
  import TranscriptSheet from './TranscriptSheet.svelte';
  import { copyCommand, DISCUSS_HERE_LABEL, DISCUSS_HINT, DISCUSS_LABEL, openSession, sessionList } from './sessions';
  import type { Sessions } from './types';

  let { sessions, folder }: { sessions: Sessions; folder: string } = $props();

  const list = $derived(sessionList(sessions));
  let note = $state<string | null>(null);
  let bad = $state(false);
  let viewing = $state<string | null>(null);
  let discussing = $state(false);

  async function open(id: string) {
    const r = await openSession(folder, id);
    bad = !r.opened && !r.copied;
    note = r.opened ? 'Opened in your terminal.' : r.message;
  }
  async function discuss(id: string) {
    await open(id);
    note = `${note} ${DISCUSS_HINT}`;
  }
  async function copy(id: string) {
    try {
      await copyCommand(folder, id);
      bad = false;
      note = 'Command copied.';
    } catch (e) {
      bad = true;
      note = (e as Error).message;
    }
  }
</script>

{#if list.length}
  <Group header="Claude Code sessions" footer={note ?? undefined} error={bad}>
    {#each list as s (s.key)}
      <Row label={s.label} sub={s.id}>
        <div class="sess-actions">
        <Button variant="tint" data-testid="session-open-{s.key}" onclick={() => open(s.id)}>Open in terminal</Button>
        {#if s.key === 'plan'}<Button variant="tint" data-testid="session-discuss" onclick={() => discuss(s.id)}>{DISCUSS_LABEL}</Button>
        <Button variant="tint" data-testid="session-discuss-here" onclick={() => { discussing = true; viewing = s.id; }}>{DISCUSS_HERE_LABEL}</Button>{/if}
        <Button variant="plain" data-testid="session-copy-{s.key}" onclick={() => copy(s.id)}>Copy command</Button>
        <Button variant="plain" data-testid="session-transcript-{s.id}" onclick={() => { discussing = false; viewing = s.id; }}>Transcript</Button>
        </div>
      </Row>
    {/each}
  </Group>
{/if}
{#if viewing}<TranscriptSheet {folder} id={viewing} discuss={discussing} onclose={() => (viewing = null)} />{/if}
