<script lang="ts">
  import type { KnowledgeKind, NodeKnowledge, Proposal } from '../api/types';
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Row from '../ui/Row.svelte';

  let { k, proposals = [], onsave, onforget, onalways, onaccept, ondismiss }: {
    k: NodeKnowledge;
    proposals?: Proposal[];
    onsave: (kind: KnowledgeKind, text: string) => void;
    onforget: (id: string) => void;
    onalways: (id: string, always: boolean) => void;
    onaccept?: (id: string) => void;
    ondismiss?: (id: string) => void;
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

{#if proposals.length}
  <Group header="Suggested procedures" footer="Accept adds it to Knowledge, so later tasks use it.">
    {#each proposals as p (p.id)}
      <Row label="When {p.applies_when}" sub="From {p.source_task}" wrap>
        <Button variant="plain" data-testid="accept-{p.id}" onclick={() => onaccept?.(p.id)}>Accept</Button>
        <Button variant="plain" data-testid="dismiss-{p.id}" onclick={() => ondismiss?.(p.id)}>Dismiss</Button>
      </Row>
      <div class="kp__steps">{p.steps}</div>
    {/each}
  </Group>
{/if}

{#if k.learned.length}
  <Group header="Learned">
    {#each k.learned as l (l.id)}
      <Row label={l.text} sub={[l.scope === 'global' && 'Global', l.always && 'Always', l.applies_when && `When ${l.applies_when}`, `x${l.count}`, l.always && l.last_confirmed && `Last confirmed ${l.last_confirmed.slice(0, 10)}`].filter(Boolean).join(' · ')} wrap>
        <Button variant="plain" data-testid="always-{l.id}" onclick={() => onalways(l.id, !l.always)}>{l.always ? 'Stop always' : 'Always'}</Button>
        <Button variant="plain" data-testid="forget-{l.id}" onclick={() => onforget(l.id)}>Forget</Button>
      </Row>
    {/each}
  </Group>
{/if}

<style>
  .kf { display: flex; flex-direction: column; gap: 4px; padding: 8px 16px; }
  .kf__label { font-weight: 600; }
  .kf .hint { margin: 0; }
  .kp__steps { padding: 0 16px 10px; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--label-2); font-size: var(--fs-13); }
  .kp__steps + :global(.row-item)::before { content: ""; position: absolute; top: 0; left: 16px; right: 0; border-top: 0.5px solid var(--sep); }
  .kf__text { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; resize: vertical; }
</style>
