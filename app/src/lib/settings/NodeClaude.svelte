<script lang="ts">
  import { createApi } from '../api/index';
  import { errText, type CcSession, type KnowledgeKind, type NodeKnowledge, type Proposal } from '../api/types';
  import { listOrNote, newestFirst, nodeFolder, openNewClaude } from '../fabric/cc';
  import { openSession, type OpenOutcome } from '../fabric/sessions';
  import TranscriptSheet from '../fabric/TranscriptSheet.svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';
  import KnowledgeGroup from './KnowledgeGroup.svelte';
  import SessionRow from './SessionRow.svelte';

  let { root, path, onadded }: { root: string; path: string; onadded?: () => void } = $props();

  let folder = $state<string | null>(null);
  let list = $state<CcSession[]>([]);
  let know = $state<NodeKnowledge | null>(null);
  let proposals = $state<Proposal[]>([]);
  let note = $state<string | null>(null);
  let bad = $state(false);
  let view = $state<CcSession | null>(null);
  let tracking = $state(false);

  const fail = (e: unknown) => { bad = true; note = errText(e); };

  $effect(() => {
    const r = root, p = path;
    folder = null;
    list = [];
    know = null;
    proposals = [];
    (async () => {
      const api = await createApi();
      const f = nodeFolder(r, p, await api.fabricHome());
      const [l, k, s] = await Promise.all([api.fabricCcSessions(f), api.fabricKnowledgeRead(f), listOrNote(api.fabricProposalsRead(f), 'Suggested procedures')]);
      folder = f;
      list = newestFirst(l);
      know = k;
      proposals = s.list;
      bad = s.note !== null;
      note = s.note;
    })().catch(fail);
  });

  async function report(go: Promise<OpenOutcome>) {
    const r = await go;
    bad = !r.opened && !r.copied;
    note = r.opened ? 'Opened in your terminal.' : r.message;
  }
  async function track(id: string) {
    if (tracking) return;
    tracking = true;
    try {
      const api = await createApi();
      await api.fabricCcTrack(folder!, id);
      list = newestFirst(await api.fabricCcSessions(folder!));
      bad = false;
      note = 'Added as a reminder';
      onadded?.();
    } catch (e) { fail(e); } finally { tracking = false; }
  }
  async function save(kind: KnowledgeKind, text: string) {
    try { know = await (await createApi()).fabricKnowledgeWrite(folder!, kind, text); note = null; bad = false; } catch (e) { fail(e); }
  }
  async function forget(id: string) {
    try { know = await (await createApi()).fabricLearnedForget(folder!, id); } catch (e) { fail(e); }
  }
  async function always(id: string, on: boolean) {
    try { know = await (await createApi()).fabricLearnedAlways(folder!, id, on); } catch (e) { fail(e); }
  }
  async function accept(id: string) {
    try {
      const api = await createApi();
      proposals = await api.fabricProposalAccept(folder!, id);
      know = await api.fabricKnowledgeRead(folder!);
    } catch (e) { fail(e); }
  }
  async function dismiss(id: string) {
    try { proposals = await (await createApi()).fabricProposalDismiss(folder!, id); } catch (e) { fail(e); }
  }
</script>

{#if folder}
  <section class="nodeset__section">
    <Group header="Claude Code" footer={note ?? undefined} error={bad}>
      {#each list as s (s.id)}
        <SessionRow {s} busy={tracking} onopen={() => report(openSession(folder!, s.id))} ontranscript={() => (view = s)} ontrack={() => track(s.id)} />
      {:else}
        <Row label="No sessions yet" />
      {/each}
      <Row label="New session here" tone="tint" onclick={() => report(openNewClaude(folder!))} />
    </Group>
    {#if know}<KnowledgeGroup k={know} {proposals} onsave={save} onforget={forget} onalways={always} onaccept={accept} ondismiss={dismiss} />{/if}
  </section>
  {#if view}<TranscriptSheet {folder} id={view.id} title={view.title} onclose={() => (view = null)} />{/if}
{/if}
