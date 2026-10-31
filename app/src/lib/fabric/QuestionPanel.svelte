<script lang="ts">
  import Button from '../ui/Button.svelte';
  import Group from '../ui/Group.svelte';
  import Segmented from '../ui/Segmented.svelte';
  import { describeAnswer, type AnswerInput } from './answer';
  import type { Question, Scope } from './types';
  import { ConflictError } from './write';

  let { question, onsubmit, onreload }: {
    question: Question;
    onsubmit?: (a: AnswerInput) => Promise<void>;
    onreload?: () => Promise<void> | void;
  } = $props();

  let text = $state('');
  let replacement = $state('');
  let refusing = $state(false);
  let scopes = $state<Record<string, Scope>>({});
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
    for (const f of question.fields ?? []) {
      const i = f.indexOf('.');
      const head = i < 0 ? f : f.slice(0, i);
      groups.set(head, [...(groups.get(head) ?? []), { field: f, leaf: i < 0 ? f : f.slice(i + 1) }]);
    }
    return [...groups];
  });

  const priorFrom = $derived.by(() => {
    const p = question.prior;
    const task = p?.source_task?.split('/').pop()?.replace(/\.md$/, '');
    return [task && `From ${task}`, p?.last_confirmed && `confirmed ${p.last_confirmed.slice(0, 10)}`].filter(Boolean).join(', ');
  });

  async function send(a: AnswerInput) {
    if (!onsubmit || busy) return;
    busy = true;
    error = '';
    conflict = false;
    try {
      await onsubmit(a);
    } catch (e) {
      conflict = e instanceof ConflictError;
      error = conflict ? 'The record changed on disk since you opened it, so nothing was written.' : (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function reload() {
    error = '';
    conflict = false;
    await onreload?.();
  }
</script>

<Group header={question.answer ? 'Question answered' : 'Question'}>
  <div class="qp">
    <p class="txt"><b>{question.id}</b> {question.text}</p>

    {#if question.answer}
      <p class="done" data-testid="answered">Answered by {question.answer.by} at {question.answer.at}: {describeAnswer(question)}</p>
      <p class="dim">Read only. The agent clears this slot when it picks the answer up.</p>
    {:else if !onsubmit}
      <p class="dim">Waiting for an answer.</p>
    {:else}
      {#if question.kind === 'open'}
        <textarea class="in" rows="3" aria-label="Answer" bind:value={text} disabled={busy}></textarea>
        <div class="acts"><Button variant="primary" disabled={busy || !text.trim()} onclick={() => send({ value: text })}>Send</Button></div>
      {:else if question.kind === 'choice'}
        {#if question.prior}
          <p class="prior">Last time: {question.prior.text}{#if priorFrom}<br /><span class="dim">{priorFrom}</span>{/if}</p>
        {/if}
        <div class="acts">
          <Button variant="primary" disabled={busy} onclick={() => send({ value: 'once' })}>Once</Button>
          <Button variant="primary" disabled={busy} onclick={() => send({ value: 'always' })}>Always</Button>
          <Button on={refusing} disabled={busy} onclick={() => (refusing = !refusing)}>No, something else</Button>
        </div>
        <p class="dim">Once is for this task. Always reuses this answer next time.</p>
        {#if refusing}
          <textarea class="in" rows="2" aria-label="Replacement" placeholder="What to do instead" bind:value={replacement} disabled={busy}></textarea>
          <div class="acts"><Button variant="primary" disabled={busy || !replacement.trim()} onclick={() => send({ value: 'no', replacement })}>Send</Button></div>
        {/if}
      {:else}
        <div class="fieldtree">
          {#each tree as [head, leaves] (head)}
            <div class="head">{head}</div>
            {#each leaves as l (l.field)}
              <div class="leaf">
                <span class="name">{l.leaf}</span>
                <Segmented size="sm" label={`Scope for ${l.field}`} options={scopeOptions} value={scopeOf(l.field)} onchange={(v) => (scopes[l.field] = v)} />
              </div>
            {/each}
          {/each}
        </div>
        <p class="dim">The agent never sees the values. It only gets to fill these fields in the form.</p>
        <div class="acts">
          <Button variant="primary" disabled={busy} onclick={() => send({ granted: (question.fields ?? []).map((f) => ({ field: f, scope: scopeOf(f) })) })}>Allow</Button>
        </div>
      {/if}
      {#if busy}<p class="dim" role="status">Saving…</p>{/if}
      {#if error}
        <p class="err" role="alert">{error}</p>
        {#if conflict}<div class="acts"><Button onclick={reload}>Reload</Button></div>{/if}
      {/if}
    {/if}
  </div>
</Group>

<style>
  .qp { padding: 10px 16px; display: flex; flex-direction: column; gap: 10px; }
  .txt { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .dim { color: var(--label-2); font-size: var(--fs-12); margin: 0; overflow-wrap: anywhere; }
  .prior { margin: 0; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); overflow-wrap: anywhere; }
  .done { margin: 0; color: var(--green); overflow-wrap: anywhere; }
  .err { margin: 0; color: var(--error-text); }
  .acts { display: flex; gap: 8px; flex-wrap: wrap; }
  .in { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border: 1px solid var(--sep); border-radius: var(--radius-ctl); background: transparent; color: inherit; }
  .fieldtree { display: flex; flex-direction: column; gap: 4px; }
  .head { font-weight: 600; margin-top: 4px; }
  .leaf { display: flex; justify-content: space-between; align-items: center; gap: 8px; flex-wrap: wrap; padding-left: 14px; }
  .name { overflow-wrap: anywhere; }
</style>
