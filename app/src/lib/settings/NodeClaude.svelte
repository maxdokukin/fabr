<script lang="ts">
  import { createApi } from '../api/index';
  import { errText, type CcSession, type KnowledgeKind, type NodeKnowledge } from '../api/types';
  import { ago, newestFirst, nodeFolder, openNewClaude, sessionTitle } from '../fabric/cc';
  import { openSession, type OpenOutcome } from '../fabric/sessions';
  import TranscriptSheet from '../fabric/TranscriptSheet.svelte';
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';
  import KnowledgeGroup from './KnowledgeGroup.svelte';

  let { root, path }: { root: string; path: string } = $props();

  let folder = $state<string | null>(null);
  let list = $state<CcSession[]>([]);
  let know = $state<NodeKnowledge | null>(null);
  let note = $state<string | null>(null);
  let bad = $state(false);
  let view = $state<CcSession | null>(null);

  const fail = (e: unknown) => { bad = true; note = errText(e); };

  $effect(() => {
    const r = root, p = path;
    folder = null;
    list = [];
    know = null;
    (async () => {
      const api = await createApi();
      const f = nodeFolder(r, p, await api.fabricHome());
      const [l, k] = await Promise.all([api.fabricCcSessions(f), api.fabricKnowledgeRead(f)]);
      folder = f;
      list = newestFirst(l);
      know = k;
    })().catch(fail);
  });

  async function report(go: Promise<OpenOutcome>) {
    const r = await go;
    bad = !r.opened && !r.copied;
    note = r.opened ? 'Opened in your terminal.' : r.message;
  }
  async function save(kind: KnowledgeKind, text: string) {
    try { know = await (await createApi()).fabricKnowledgeWrite(folder!, kind, text); note = null; bad = false; } catch (e) { fail(e); }
  }
  async function forget(id: string) {
    try { know = await (await createApi()).fabricLearnedForget(folder!, id); } catch (e) { fail(e); }
  }
</script>

{#if folder}
  <section class="nodeset__section">
    <Group header="Claude Code" footer={note ?? undefined} error={bad}>
      {#each list as s (s.id)}
        <Row label={sessionTitle(s)} sub="{ago(s.modified_at)} · {s.messages} messages">
          <Button variant="tint" data-testid="cc-open-{s.id}" onclick={() => report(openSession(folder!, s.id))}>Open</Button>
          <Button variant="plain" data-testid="cc-transcript-{s.id}" onclick={() => (view = s)}>Transcript</Button>
        </Row>
      {:else}
        <Row label="No sessions yet" />
      {/each}
      <Row label="New session here" tone="tint" onclick={() => report(openNewClaude(folder!))} />
    </Group>
    {#if know}<KnowledgeGroup k={know} onsave={save} onforget={forget} />{/if}
  </section>
  {#if view}<TranscriptSheet {folder} id={view.id} title={view.title} onclose={() => (view = null)} />{/if}
{/if}
