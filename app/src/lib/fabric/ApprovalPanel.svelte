<script lang="ts">
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import type { ApprovalDecision } from './approval';
  import type { Approval, Assertion, Grants, PlanMeta, Scope } from './types';
  import TranscriptSheet from './TranscriptSheet.svelte';
  import { DISCUSS_HERE_LABEL, DISCUSS_HINT, DISCUSS_LABEL, openSession } from './sessions';
  import { ConflictError } from './write';
  import Prose from './Prose.svelte';

  let { approval, grants, planMeta, plan, assertions, onsubmit, onreload, folder = '', planSession = null, onredo }: {
    approval: Approval;
    grants: Grants;
    planMeta: PlanMeta | null;
    plan: string;
    assertions: Assertion[];
    onsubmit?: (d: ApprovalDecision) => Promise<void>;
    onreload?: () => Promise<void> | void;
    folder?: string;
    planSession?: string | null;
    onredo?: () => Promise<void> | void;
  } = $props();

  let scopes = $state<Record<string, Scope>>({});
  let skipped = $state<Record<string, boolean>>({});
  let refusing = $state(false);
  let note = $state('');
  let busy = $state(false);
  let error = $state('');
  let conflict = $state(false);
  let discussed = $state('');
  let discussHere = $state(false);

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
  const BASE = ['Read', 'Glob', 'Grep'];
  const FILE_TOOLS = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit'];
  const ruleNote = (rule: string): { note: string; hot: boolean } => {
    const name = rule.split('(')[0];
    if (name === 'Bash') return { note: 'runs commands', hot: true };
    if (name === 'WebFetch' || name === 'WebSearch') return { note: 'uses the network', hot: true };
    if (!FILE_TOOLS.includes(name)) return { note: '', hot: false };
    const path = rule.includes('(') ? rule.slice(rule.indexOf('(') + 1).replace(/\)$/, '').trim() : '';
    const outside = /^(\/|~)/.test(path) || path.includes('..');
    return outside ? { note: 'outside this folder', hot: true } : { note: 'inside this folder', hot: false };
  };
  const assumed = $derived(planMeta?.assumptions ?? []);
  const rules = $derived((planMeta?.tools ?? []).filter((t) => !BASE.includes(t)).map((t) => ({ t, ...ruleNote(t) })));
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

  async function discuss() {
    const r = await openSession(folder, planSession!);
    discussed = r.opened ? 'Opened in your terminal.' : r.message;
  }

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
        {#if plan}<div class="txt"><Prose text={plan} /></div>{:else}<p class="dim">No plan text yet.</p>{/if}
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
        <section data-testid="may-use">
          <h4>Claude Code may use</h4>
          <div class="chips">
            {#each BASE as t (t)}<span class="chip muted">{t}</span>{/each}
            {#each rules as r (r.t)}<span class="chip" class:hot={r.hot}>{r.t}{#if r.note}<small class:flag={r.hot}>{r.note}</small>{/if}</span>{/each}
          </div>
          {#if planMeta.checks?.length}
            <p class="dim">Checks after the run:</p>
            <ul class="list mono">{#each planMeta.checks as c, i (i)}<li>{[c.cmd, ...c.args].join(' ')}</li>{/each}</ul>
          {/if}
        </section>
      {/if}

      {#if assumed.length}
        <section data-testid="assumed">
          <h4>Assumed</h4>
          <ul class="list">{#each assumed as a (a.rule_id + a.text)}<li>{a.text}</li>{/each}</ul>
          <p class="dim">Applied because you chose Always.</p>
        </section>
      {/if}

      {#if planSession && folder}
        <section>
          <div class="acts acts--text">
            <Button variant="tint" data-testid="discuss-plan" onclick={discuss}>{DISCUSS_LABEL}</Button>
            <Button variant="tint" data-testid="discuss-here" onclick={() => (discussHere = true)}>{DISCUSS_HERE_LABEL}</Button>
            {#if onredo}<Button variant="tint" disabled={busy} data-testid="approval-redo-plan" onclick={onredo}>Redo plan</Button>{/if}
          </div>
          <p class="dim" data-testid="discuss-hint">{discussed ? `${discussed} ` : ''}{DISCUSS_HINT}</p>
        </section>
      {/if}

      {#if irreversible}<p class="warn" role="alert" data-testid="irreversible">Irreversible: once this runs it cannot be undone.</p>{/if}

      {#if !onsubmit}
        <p class="dim">Waiting for a decision.</p>
      {:else}
        <div class="acts">
          <Button variant="primary" disabled={busy} onclick={approve}>Approve</Button>
          <Button variant="danger" on={refusing} disabled={busy} onclick={() => (refusing = !refusing)}>Refuse</Button>
        </div>
        {#if refusing}
          <textarea class="in" rows="2" aria-label="Refusal note" placeholder="Why (optional)" bind:value={note} disabled={busy}></textarea>
          <div class="acts"><Button variant="danger" disabled={busy} onclick={refuse}>Confirm refusal</Button></div>
        {/if}
        {#if busy}<p class="dim" role="status">Saving…</p>{/if}
        {#if error}
          <p class="err" role="alert">{error}</p>
          {#if conflict}<div class="acts"><Button onclick={reload}>Reload</Button></div>{/if}
        {/if}
      {/if}
    {:else}
      <p class="txt" data-testid="decided"><b class:warn={approval.status === 'refused'}>{approval.status}</b>{#if approval.by}{" "}by {approval.by}{/if}{#if approval.at}{" "}at {approval.at}{/if}</p>
      {#if approval.note}<p class="dim">Note: {approval.note}</p>{/if}
      <p class="txt">granted: {grants.granted.map(describe).join(', ') || 'none'}</p>
      <p class="dim">requested: {grants.requested.join(', ') || 'none'}</p>
    {/if}
  </div>
</Group>
{#if discussHere && planSession}<TranscriptSheet {folder} id={planSession} discuss={true} onclose={() => (discussHere = false)} />{/if}

<style>
  .ap { padding: 10px 16px; display: flex; flex-direction: column; gap: 12px; }
  section { display: flex; flex-direction: column; gap: 4px; }
  h4 { margin: 0; font-size: var(--fs-12); color: var(--label-2); text-transform: uppercase; }
  .txt { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .dim { color: var(--label-2); font-size: var(--fs-12); margin: 0; overflow-wrap: anywhere; }
  .warn { color: var(--warning-text); font-weight: 600; margin: 0; }
  .err { margin: 0; color: var(--error-text); }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; overflow-wrap: anywhere; }
  .acts { display: flex; gap: 8px; flex-wrap: wrap; }
  .acts--text { margin-left: -10px; }
  .in { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; }
  .fieldtree { display: flex; flex-direction: column; gap: 4px; }
  .head { font-weight: 600; margin-top: 4px; }
  .leaf { display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; padding-left: 14px; }
  .name { overflow-wrap: anywhere; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: var(--fs-12); padding: 2px 6px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); overflow-wrap: anywhere; }
  .chip.muted { color: var(--label-2); }
  .chip.hot { border-color: var(--orange); }
  .chip small { color: var(--label-2); margin-left: 4px; }
  .chip small.flag { color: var(--warning-text); }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: var(--fs-12); }
</style>
