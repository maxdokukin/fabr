<script lang="ts">
  import type { KnowledgeKind, NodeKnowledge } from '../api/types';
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';

  let { k, onsave, onforget, onalways }: {
    k: NodeKnowledge;
    onsave: (kind: KnowledgeKind, text: string) => void;
    onforget: (id: string) => void;
    onalways: (id: string, always: boolean) => void;
  } = $props();

  const FIELDS: { kind: KnowledgeKind; label: string; hint: string }[] = [
    { kind: 'rules', label: 'Rules', hint: 'How the agent must behave in this folder' },
    { kind: 'knowledge', label: 'Knowledge', hint: 'Facts about this folder' },
  ];
  let edits = $state<Partial<Record<KnowledgeKind, string>>>({});
  const text = (kind: KnowledgeKind) => edits[kind] ?? k[kind];
  $effect(() => { void k; edits = {}; });
</script>

<Group header="Knowledge">
  {#each FIELDS as f (f.kind)}
    <div class="kf">
      <span class="kf__label">{f.label}</span>
      <textarea class="kf__text" rows="4" aria-label={f.label} value={text(f.kind)} oninput={(e) => (edits[f.kind] = e.currentTarget.value)} onblur={() => text(f.kind) !== k[f.kind] && onsave(f.kind, text(f.kind))}></textarea>
      <span class="hint">{f.hint}</span>
    </div>
  {/each}
</Group>

{#if k.learned.length}
  <Group header="Learned">
    {#each k.learned as l (l.id)}
      <Row label={l.text} sub={[l.always && 'Always', l.applies_when && `When ${l.applies_when}`, `x${l.count}`].filter(Boolean).join(' · ')} wrap>
        <Button variant="plain" data-testid="always-{l.id}" onclick={() => onalways(l.id, !l.always)}>{l.always ? 'Stop always' : 'Always'}</Button>
        <Button variant="plain" data-testid="forget-{l.id}" onclick={() => onforget(l.id)}>Forget</Button>
      </Row>
    {/each}
  </Group>
{/if}

<style>
  .kf { display: flex; flex-direction: column; gap: 4px; padding: 8px 16px; }
  .kf__label { font-weight: 600; }
  .kf__text { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; resize: vertical; }
</style>
