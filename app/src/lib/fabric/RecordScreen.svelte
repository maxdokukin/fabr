<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Page from '../ui/Page.svelte';
  import type { AnswerInput } from './answer';
  import ApprovalPanel from './ApprovalPanel.svelte';
  import type { ApprovalDecision } from './approval';
  import { describeAnswer } from './answer';
  import { splitContext } from './parse';
  import QuestionPanel from './QuestionPanel.svelte';
  import { BACKEND_LABEL } from './backend';
  import { backendSilent, canComplete, canPauseResume, canRedo, COMPLETE_PENDING, isRedoKey, pendingRequests, requestFor, type RequestKey } from './requests';
  import { editStart, isEditable, provenance, REDO_LABEL, REDO_PENDING, type FieldsEdit, type Section } from './sections';
  import { canShelve, isOpen, phaseOf } from './phase';
  import { suspendLabel } from './stage';
  import SpeechCapture from './SpeechCapture.svelte';
  import StateRibbon from './StateRibbon.svelte';
  import { minuteOf } from '../model';
  import type { TaskRecord } from './types';

  let { record, path = '', name, backLabel = 'Records', agent, onback, onanswer, onapprove, onrequest, onedit, onfields, onsuspend, onunsuspend, now = new Date(), onreload, readonly = false }: {
    record: TaskRecord;
    path?: string;
    name: string;
    backLabel?: string;
    agent?: Snippet;
    onback?: () => void;
    onanswer?: (a: AnswerInput) => Promise<void>;
    onapprove?: (d: ApprovalDecision) => Promise<void>;
    onrequest?: (key: RequestKey) => Promise<void>;
    onedit?: (section: Section | 'assertions', text: string) => Promise<void>;
    onfields?: (f: FieldsEdit) => Promise<void>;
    onsuspend?: (until: string | null) => Promise<void>;
    onunsuspend?: () => Promise<void>;
    now?: Date | string;
    onreload?: () => Promise<void> | void;
    readonly?: boolean;
  } = $props();

  const labels: Record<RequestKey, string> = { delegate: 'Delegate', cancel: 'Cancel', redelegate: 'Re-delegate', pause: 'Pause', resume: 'Resume', complete: 'Mark done', redo_define: REDO_LABEL.define, redo_plan: REDO_LABEL.plan };
  const wanted = $derived(requestFor(record.state));
  const canAsk = $derived(wanted !== null && !record.requests[wanted] && !(wanted === 'redelegate' && record.links.redelegated_to));
  const queued = $derived(pendingRequests(record).filter((k) => k !== 'pause' && k !== 'resume' && k !== 'complete' && !isRedoKey(k)));
  const phase = $derived(phaseOf(record, minuteOf(now)));
  const flip = $derived(canPauseResume(record, phase));
  const flipping = $derived(record.requests.pause ? 'pause' : record.requests.resume ? 'resume' : null);
  const mayComplete = $derived(canComplete(record, phase));
  const shelved = $derived(phase === 'suspended');
  const mayShelve = $derived(canShelve(phase));
  let picking = $state(false);
  let mode = $state<'none' | 'date' | 'time'>('none');
  let day = $state('');
  let clock = $state('');
  const until = $derived(mode === 'none' ? null : mode === 'date' || !clock ? day : `${day}T${clock}`);
  async function shelve() {
    busy = true;
    reqError = null;
    try { await onsuspend?.(until); picking = false; mode = 'none'; day = ''; clock = ''; }
    catch (e) { reqError = (e as Error).message; }
    finally { busy = false; }
  }
  async function unshelve() {
    busy = true;
    reqError = null;
    try { await onunsuspend?.(); }
    catch (e) { reqError = (e as Error).message; }
    finally { busy = false; }
  }
  const mayRedo = $derived(canRedo(record) && !record.suspended);
  const editable = $derived(!readonly && isEditable(record) && !!onedit);
  const holding = $derived(editStart(record, phase) === 'pause');
  type Slot = Section | 'assertions' | 'fields';
  let editing = $state<Slot | null>(null);
  let wantedSlot = $state<Slot | null>(null);
  let draft = $state('');
  let fTitle = $state('');
  let fDue = $state('');
  let fPri = $state(0);
  const pausing = $derived(wantedSlot !== null && editing === null);
  const textOf = (slot: Slot): string => (slot === 'assertions' ? record.assertions.map((a) => a.text).join('\n') : slot === 'fields' ? '' : record.sections[slot]);
  function open(slot: Slot) {
    draft = textOf(slot);
    fTitle = record.title;
    fDue = record.due ?? '';
    fPri = record.pri;
    reqError = null;
    editing = slot;
  }
  async function edit(slot: Slot) {
    if (!holding) return open(slot);
    wantedSlot = slot;
    if (record.requests.pause) return;
    await ask('pause');
    if (reqError) wantedSlot = null;
  }
  $effect(() => {
    if (wantedSlot !== null && editing === null && phase !== 'working') {
      const slot = wantedSlot;
      wantedSlot = null;
      open(slot);
    }
  });
  async function save() {
    if (editing === null) return;
    busy = true;
    reqError = null;
    try {
      if (editing === 'fields') await onfields?.({ title: fTitle, due: fDue || null, pri: fPri });
      else await onedit?.(editing, draft);
      editing = null;
    } catch (e) { reqError = (e as Error).message; }
    finally { busy = false; }
  }
  const redoPending = $derived(record.requests.redo === 'define' || record.requests.redo === 'plan');
  const silent = $derived(backendSilent(record, now));
  let busy = $state(false);
  let reqError = $state<string | null>(null);
  async function ask(key: RequestKey) {
    busy = true;
    reqError = null;
    try { await onrequest?.(key); }
    catch (e) { reqError = (e as Error).message; }
    finally { busy = false; }
  }

  const s = $derived(record.sections);
  const ctx = $derived(splitContext(s.context));
  const items = $derived(new Map((record.verdict?.items ?? []).map((i) => [i.id, i])));
  const mark = (id: string) => {
    const i = items.get(id);
    return i ? (i.pass ? 'pass' : 'fail') : 'unchecked';
  };
  const symbol = { pass: '✓', fail: '✗', unchecked: '–' } as const;
  const q = $derived(record.question);
  const ap = $derived(record.approval);
  const pm = $derived(record.plan_meta);
</script>

{#snippet sechead(slot: Section | 'assertions')}
  {@const who = slot === 'assertions' ? provenance(record, 'success') : provenance(record, slot)}
  <div class="sechead">
    {#if who}<span class="prov prov--{who}" data-testid="prov-{slot}">{who}</span>{/if}
    {#if slot === 'spec' || slot === 'success' || slot === 'assertions'}{#if redoPending && record.requests.redo === 'plan'}<span class="req__queued" data-testid="redo-pending">{REDO_PENDING}</span>{/if}{/if}
    {#if editable && editing === null}
      <Button variant="plain" disabled={busy || pausing} data-testid="edit-{slot}" onclick={() => edit(slot)}>{wantedSlot === slot ? 'pausing…' : 'Edit'}</Button>
    {/if}
  </div>
{/snippet}

{#snippet editor(slot: Slot)}
  {#if editing === slot}
    <div class="editor" data-testid="editor-{slot}">
      {#if slot === 'fields'}
        <input class="editor__in" type="text" aria-label="Title" bind:value={fTitle} />
        <input class="editor__in" type="text" aria-label="Due" placeholder="YYYY-MM-DD or YYYY-MM-DDTHH:MM" bind:value={fDue} />
        <select class="editor__in" aria-label="Priority" bind:value={fPri}>{#each [0, 1, 2, 3] as n (n)}<option value={n}>{n === 0 ? 'No priority' : '!'.repeat(n)}</option>{/each}</select>
      {:else}
        <textarea class="editor__text" rows={slot === 'assertions' ? 4 : 8} aria-label={slot === 'assertions' ? 'Assertions, one per line' : `Edit ${slot}`} bind:value={draft}></textarea>
        {#if slot === 'assertions'}<p class="dim">One assertion per line.</p>{/if}
      {/if}
      <div class="req">
        <Button variant="primary" disabled={busy} data-testid="save-{slot}" onclick={save}>Save</Button>
        <Button variant="plain" disabled={busy} onclick={() => (editing = null)}>Close</Button>
        {#if reqError}<span class="req__err">{reqError}</span>{/if}
      </div>
    </div>
  {/if}
{/snippet}

<Page title={record.title} back={backLabel} {onback}>
  <div class="rec">
    <header class="rec__head">
      <p class="rec__meta">{name}{#if record.due} · due {record.due}{/if} · cost ${record.cost_usd}{#if record.pri} · {'!'.repeat(record.pri)}{/if}</p>
      {#if editable && editing === null}<Button variant="plain" disabled={busy} data-testid="edit-fields" onclick={() => open('fields')}>Edit title, due, priority</Button>{/if}
      {@render editor('fields')}
    </header>

    <StateRibbon state={record.state} step={record.step} {phase} needs={record.needs} />

    {#if agent}{@render agent()}{/if}

    {#if !readonly && (canAsk || mayRedo || redoPending || queued.length || flip || flipping || mayComplete || record.requests.complete || shelved || mayShelve)}
      <div class="req">
        {#if canAsk && wanted}
          <Button variant={wanted === 'cancel' ? 'danger' : 'primary'} disabled={busy} onclick={() => ask(wanted)}>{labels[wanted]}</Button>
        {/if}
        {#if mayComplete}<Button variant="tint" disabled={busy} data-testid="mark-done" onclick={() => ask('complete')}>{labels.complete}</Button>{/if}
        {#if record.requests.complete}<span class="req__queued" data-testid="complete-pending">{COMPLETE_PENDING}</span>{/if}
        {#if flip}<Button variant="tint" disabled={busy} data-testid="pause-resume" onclick={() => ask(flip)}>{labels[flip]}</Button>{/if}
        {#if flipping}<span class="req__queued">{flipping} requested</span>{/if}
        {#if mayRedo}
          <Button variant="tint" disabled={busy} data-testid="redo-plan" onclick={() => ask('redo_plan')}>{labels.redo_plan}</Button>
          <Button variant="tint" disabled={busy} data-testid="redo-define" onclick={() => ask('redo_define')}>{labels.redo_define}</Button>
        {/if}
        {#if redoPending}<span class="req__queued" data-testid="redo-pending-top">{REDO_PENDING}</span>{/if}
        {#if shelved}
          <span class="req__queued">{suspendLabel(record)}</span>
          <Button variant="tint" disabled={busy} data-testid="unsuspend" onclick={unshelve}>Unsuspend</Button>
        {:else if mayShelve && !picking}
          <Button variant="tint" disabled={busy} data-testid="suspend" onclick={() => (picking = true)}>Suspend…</Button>
        {/if}
        {#each queued as k (k)}<span class="req__queued">{labels[k]}: queued</span>{/each}
        {#if silent}<span class="req__silent">{BACKEND_LABEL.absent}</span>{/if}
        {#if reqError}<span class="req__err">{reqError}</span>{/if}
      </div>
    {/if}

    {#if !readonly && picking && !shelved}
      <div class="req" data-testid="suspend-picker">
        <select bind:value={mode} aria-label="Suspend until">
          <option value="none">No end time</option>
          <option value="date">Until a date</option>
          <option value="time">Until a date and time</option>
        </select>
        {#if mode !== 'none'}<input type="date" bind:value={day} aria-label="Until date" />{/if}
        {#if mode === 'time'}<input type="time" bind:value={clock} aria-label="Until time" />{/if}
        <Button variant="primary" disabled={busy || (mode !== 'none' && !day)} data-testid="suspend-confirm" onclick={shelve}>Suspend</Button>
        <Button variant="plain" disabled={busy} onclick={() => (picking = false)}>Close</Button>
      </div>
    {/if}

    <Group header="Context">
      <div class="blk">
        {@render sechead('context')}
        {@render editor('context')}
        {#if editing === 'context'}{:else if ctx.own}<p class="txt">{ctx.own}</p>{:else}<p class="slot">Context: written by you when the task is created</p>{/if}
        {#if ctx.gathered}<h4 class="sub">Gathered</h4><p class="txt">{ctx.gathered}</p>
        {:else}<p class="slot">Gathered: appended by the gatherer during gather</p>{/if}
      </div>
    </Group>

    <Group header="Spec">
      <div class="blk">
        {@render sechead('spec')}
        {@render editor('spec')}
        {#if editing === 'spec'}{:else if s.spec}<p class="txt">{s.spec}</p>{:else}<p class="slot">Spec: written by the definer during define</p>{/if}
      </div>
    </Group>

    <Group header="Success">
      <div class="blk">
        {@render sechead('success')}
        {@render editor('success')}
        {#if editing === 'success'}{:else if s.success}<p class="txt">{s.success}</p>{:else}<p class="slot">Success: the success definition, written by you</p>{/if}
        {#if editable && editing === null}<div class="sechead"><Button variant="plain" disabled={busy || pausing} data-testid="edit-assertions" onclick={() => edit('assertions')}>Edit assertions</Button></div>{/if}
        {@render editor('assertions')}
        {#if editing === 'assertions'}{:else if record.assertions.length}
          <ul class="list">
            {#each record.assertions as a (a.id)}
              <li class="assert"><span class="mark mark--{mark(a.id)}" title={mark(a.id)}>{symbol[mark(a.id)]}</span><span><b>{a.id}</b> {a.text} <span class="dim">({mark(a.id)})</span></span></li>
            {/each}
          </ul>
        {:else}<p class="slot">Assertions: the checkable form, written by the definer during define</p>{/if}
      </div>
    </Group>

    {#if q && !readonly}
      <QuestionPanel question={q} onsubmit={onanswer} {onreload} />
    {/if}

    {#if record.answers.length}
      <Group header="Answers">
        <div class="blk">
          <ul class="list">
            {#each record.answers as a (a.id)}
              <li class="txt"><b>{a.id}</b> ({a.kind}) {a.text}<br /><span class="dim">{describeAnswer(a)}{#if a.answer} · by {a.answer.by} at {a.answer.at}{/if}</span></li>
            {/each}
          </ul>
        </div>
      </Group>
    {/if}

    <Group header="Plan">
      <div class="blk">
        {@render sechead('plan')}
        {@render editor('plan')}
        {#if editing === 'plan'}{:else if s.plan}<p class="txt">{s.plan}</p>{:else}<p class="slot">Plan: written by the planner during plan</p>{/if}
        {#if pm}
          <p class="dim">reversibility: <b class:warn={pm.reversibility === 'irreversible'}>{pm.reversibility}</b> · tools: {pm.tools.join(', ') || 'none'}</p>
          {#if pm.assumptions.length}
            <ul class="list">{#each pm.assumptions as a (a.rule_id)}<li>assumes <b>{a.rule_id}</b>: {a.text}</li>{/each}</ul>
          {/if}
        {/if}
      </div>
    </Group>

    {#if ap && !readonly}
      <ApprovalPanel approval={ap} grants={record.grants} planMeta={pm} plan={s.plan} assertions={record.assertions} onsubmit={onapprove} {onreload} />
    {:else}
      <Group header="Grants">
        <div class="blk">
          <p class="txt">requested: {record.grants.requested.join(', ') || 'none'}</p>
          <p class="txt">granted: {record.grants.granted.map((g) => `${g.field} (${g.scope})`).join(', ') || 'none'}</p>
        </div>
      </Group>
      <Group header="Approval">
        <div class="blk"><p class="slot">Approval: requested by the backend after plan</p></div>
      </Group>
    {/if}

    {#if path}
      <SpeechCapture {path} log={s.log} />
    {/if}

    <Group header="Log">
      <div class="blk">
        {#if s.logEntries.length}
          <ul class="list">
            {#each s.logEntries as e, i (i)}<li class="log"><span class="dim">{e.ts}</span> <b>{e.step}</b> {e.message}</li>{/each}
          </ul>
        {:else}<p class="slot">Log: written by the backend as each step runs</p>{/if}
      </div>
    </Group>

    <Group header="Verdict">
      <div class="blk">
        {#if s.logEntries.some((e) => e.message.includes('UNKNOWN'))}
          <p class="warn" data-testid="unknown-outcome">Failed. Waiting for reconciliation. This is not a success.</p>
        {/if}
        {#if record.verdict}
          <p class="txt"><b class:warn={record.verdict.result === 'fail' || s.logEntries.some((e) => e.message.includes('UNKNOWN'))}>{record.verdict.result}</b> at {record.verdict.at}</p>
          <ul class="list">
            {#each record.verdict.items as i (i.id)}
              <li class="assert"><span class="mark mark--{i.pass ? 'pass' : 'fail'}">{i.pass ? '✓' : '✗'}</span><span><b>{i.id}</b> {i.evidence}</span></li>
            {/each}
          </ul>
        {:else}<p class="slot">Verdict: written by the backend during evaluate</p>{/if}
      </div>
    </Group>

    {#if record.links.redelegated_from || record.links.redelegated_to}
      <Group header="Links">
        <div class="blk">
          {#if record.links.redelegated_from}<p class="txt">re-delegated from {record.links.redelegated_from}</p>{/if}
          {#if record.links.redelegated_to}<p class="txt">re-delegated to {record.links.redelegated_to}</p>{/if}
        </div>
      </Group>
    {/if}
  </div>
</Page>

<style>
  .rec { display: flex; flex-direction: column; gap: 18px; padding: 8px 0 24px; }
  .rec__meta { margin: 4px 0 0; font-size: var(--fs-12); color: var(--label-2); }
  .blk { padding: 10px 16px; display: flex; flex-direction: column; gap: 6px; }
  .txt { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .dim { color: var(--label-2); font-size: var(--fs-12); margin: 0; }
  .slot { margin: 0; color: var(--label-3); font-style: italic; border: 1px dashed var(--sep); border-radius: var(--radius-ctl); padding: 6px 10px; }
  .sub { margin: 4px 0 0; font-size: var(--fs-12); color: var(--label-2); text-transform: uppercase; }
  .list { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; }
  .assert { display: flex; gap: 8px; }
  .mark { width: 1.2em; text-align: center; font-weight: 700; color: var(--label-3); }
  .mark--pass { color: var(--green); }
  .mark--fail { color: var(--red); }
  .req { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
  .req__queued { color: var(--label-2); font-size: var(--fs-12); }
  .req__silent { color: var(--orange); font-size: var(--fs-12); }
  .req__err { color: var(--red); font-size: var(--fs-12); }
  .warn { color: var(--orange); }
  .sechead { display: flex; align-items: center; justify-content: flex-end; gap: 10px; min-height: 0; }
  .prov { margin-right: auto; font-size: var(--fs-11); text-transform: uppercase; color: var(--label-3); border: 0.5px solid var(--sep); border-radius: var(--radius-pill); padding: 1px 8px; }
  .prov--you { color: var(--tint); border-color: var(--tint); }
  .editor { display: flex; flex-direction: column; gap: 8px; }
  .editor__text, .editor__in { width: 100%; font: inherit; color: var(--label); background: var(--bg-elev); border: 0.5px solid var(--sep); border-radius: var(--radius-ctl); padding: 8px 10px; }
  .editor__text { resize: vertical; }
  .log { font-size: var(--fs-12); overflow-wrap: anywhere; }
</style>
