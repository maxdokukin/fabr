<script lang="ts">
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import type { ApprovalDecision } from './approval';
  import type { Approval, Assertion, Grants, PlanMeta, Scope } from './types';
  import { ConflictError } from './write';

  let { approval, grants, planMeta, plan, assertions, onsubmit, onreload }: {
    approval: Approval;
    grants: Grants;
    planMeta: PlanMeta | null;
    plan: string;
    assertions: Assertion[];
    onsubmit?: (d: ApprovalDecision) => Promise<void>;
    onreload?: () => Promise<void> | void;
  } = $props();

  let scopes = $state<Record<string, Scope>>({});
  let skipped = $state<Record<string, boolean>>({});
  let refusing = $state(false);
  let note = $state('');
  let busy = $state(false);
  let error = $state('');
  let conflict = $state(false);

  const scopeOf = (f: string): Scope => scopes[f] ?? 'task';
  const scopeOptions: { value: Scope; label: string }[] = [
    { value: 'task', label: 'This task' },
    { value: 'project', label: 'This project' },
    { value: 'always', label: 'Always' },
  ];
  const tree = $derived.by(() => {
    const groups = new Map<string, { field: string; leaf: string }[]>();
    for (const f of grants.requested) {
      const i = f.indexOf('.');
      const head = i < 0 ? f : f.slice(0, i);
      groups.set(head, [...(groups.get(head) ?? []), { field: f, leaf: i < 0 ? f : f.slice(i + 1) }]);
    }
    return [...groups];
  });
  const irreversible = $derived(planMeta?.reversibility === 'irreversible');
  const describe = (g: { field: string; scope: Scope }) => `${g.field} (${g.scope})`;

  async function send(d: ApprovalDecision) {
    if (!onsubmit || busy) return;
    busy = true;
    error = '';
    conflict = false;
    try {
      await onsubmit(d);
    } catch (e) {
      conflict = e instanceof ConflictError;
      error = conflict ? 'The record changed on disk since you opened it, so nothing was written.' : (e as Error).message;
    } finally {
      busy = false;
    }
  }

  const approve = () => send({ status: 'approved', granted: grants.requested.filter((f) => !skipped[f]).map((f) => ({ field: f, scope: scopeOf(f) })) });
  const refuse = () => send({ status: 'refused', note });

  async function reload() {
    error = '';
    conflict = false;
    await onreload?.();
  }
</script>

<Group header={approval.status === 'pending' ? 'Approval needed' : 'Approval'}>
  <div class="ap">
    {#if approval.status === 'pending'}
      <section>
        <h4>Plan</h4>
        {#if plan}<p class="txt">{plan}</p>{:else}<p class="dim">No plan text yet.</p>{/if}
      </section>

      <section>
        <h4>Success assertions</h4>
        <ul class="list">
          {#each assertions as a (a.id)}<li><b>{a.id}</b> {a.text}</li>{/each}
        </ul>
      </section>

      <section>
        <h4>Fields the agent may fill</h4>
        {#if tree.length}
          <div class="fieldtree">
            {#each tree as [head, leaves] (head)}
              <div class="head">{head}</div>
              {#each leaves as l (l.field)}
                <div class="leaf">
                  <label class="name"><input type="checkbox" checked={!skipped[l.field]} disabled={busy || !onsubmit} onchange={(e) => (skipped[l.field] = !e.currentTarget.checked)} /> {l.leaf}</label>
                  <Segmented size="sm" label={`Scope for ${l.field}`} options={scopeOptions} value={scopeOf(l.field)} onchange={(v) => (scopes[l.field] = v)} />
                </div>
              {/each}
            {/each}
          </div>
          <p class="dim">The agent never sees the values. Unticked fields are not granted.</p>
        {:else}<p class="dim">No fields requested.</p>{/if}
      </section>

      {#if planMeta}
        <section>
          <h4>Reversibility</h4>
          {#if irreversible}
            <p class="warn" role="alert" data-testid="irreversible">Irreversible: once this runs it cannot be undone.</p>
          {:else}<p class="txt">reversible</p>{/if}
          <p class="dim">tools: {planMeta.tools.join(', ') || 'none'}</p>
          {#if planMeta.bound_actions?.length}
            <h4>Bound actions</h4>
            <ul class="list" data-testid="bound-actions">
              {#each planMeta.bound_actions as a, i (i)}
                <li><b>{a.tool}</b> {a.destination} · {a.effect} · {a.grant_class}</li>
              {/each}
            </ul>
          {/if}
        </section>

        {#if planMeta.assumptions.length}
          <section>
            <h4>Assumptions</h4>
            <p class="dim">These always-rules were applied without asking.</p>
            <ul class="list">{#each planMeta.assumptions as a (a.rule_id)}<li><b>{a.rule_id}</b>: {a.text}</li>{/each}</ul>
          </section>
        {/if}
      {/if}

      {#if !onsubmit}
        <p class="dim">Waiting for a decision.</p>
      {:else}
        <div class="row">
          <Button variant="primary" disabled={busy} onclick={approve}>Approve</Button>
          <Button variant="danger" on={refusing} disabled={busy} onclick={() => (refusing = !refusing)}>Refuse</Button>
        </div>
        {#if refusing}
          <textarea class="in" rows="2" aria-label="Refusal note" placeholder="Why (optional)" bind:value={note} disabled={busy}></textarea>
          <div class="row"><Button variant="danger" disabled={busy} onclick={refuse}>Confirm refusal</Button></div>
        {/if}
        {#if busy}<p class="dim" role="status">Saving…</p>{/if}
        {#if error}
          <p class="err" role="alert">{error}</p>
          {#if conflict}<div class="row"><Button onclick={reload}>Reload</Button></div>{/if}
        {/if}
      {/if}
    {:else}
      <p class="txt" data-testid="decided"><b class:warn={approval.status === 'refused'}>{approval.status}</b>{#if approval.by} by {approval.by}{/if}{#if approval.at} at {approval.at}{/if}</p>
      {#if approval.note}<p class="dim">Note: {approval.note}</p>{/if}
      <p class="txt">granted: {grants.granted.map(describe).join(', ') || 'none'}</p>
      <p class="dim">requested: {grants.requested.join(', ') || 'none'}</p>
    {/if}
  </div>
</Group>

<style>
  .ap { padding: 10px 16px; display: flex; flex-direction: column; gap: 12px; }
  section { display: flex; flex-direction: column; gap: 4px; }
  h4 { margin: 0; font-size: var(--fs-12); color: var(--label-2); text-transform: uppercase; }
  .txt { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .dim { color: var(--label-2); font-size: var(--fs-12); margin: 0; overflow-wrap: anywhere; }
  .warn { color: var(--orange); font-weight: 600; margin: 0; }
  .err { margin: 0; color: var(--red); }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; overflow-wrap: anywhere; }
  .row { display: flex; gap: 8px; flex-wrap: wrap; }
  .in { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; }
  .fieldtree { display: flex; flex-direction: column; gap: 4px; }
  .head { font-weight: 600; margin-top: 4px; }
  .leaf { display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; padding-left: 14px; }
  .name { overflow-wrap: anywhere; }
</style>
