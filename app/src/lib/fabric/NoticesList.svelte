<script lang="ts">
  import { isUrgent, markAllRead, taskIdOf, unreadCount, type FabricEvent } from './events';
  let { events, readPosition, titleFor, outOfScope, onopen, onmarkallread }: {
    events: FabricEvent[];
    readPosition: number;
    titleFor?: (taskPath: string) => string | undefined;
    outOfScope?: (taskPath: string) => boolean;
    onopen?: (taskPath: string) => void;
    onmarkallread?: (position: number) => void;
  } = $props();

  const unread = $derived(unreadCount(events, readPosition));
  const titleOf = (p: string) => titleFor?.(p) || taskIdOf(p);
  const isOutOfScope = (p: string) => outOfScope?.(p) ?? false;
</script>

<section class="nlist" aria-label="Notices">
  <header class="nlist__head">
    <span>{unread} unread</span>
    <button type="button" class="nlist__mark" disabled={unread === 0} onclick={() => onmarkallread?.(markAllRead(events))}>mark all read</button>
  </header>
  {#if events.length === 0}
    <p class="nlist__empty">No notices</p>
  {:else}
    <ul class="nlist__rows">
      {#each events as e, i (i)}
        {@const oos = isOutOfScope(e.taskPath)}
        <li>
          <button type="button" class="nrow nrow--{e.kind}" class:nrow--urgent={isUrgent(e.kind)} class:nrow--unread={i < unread} class:nrow--oos={oos} onclick={() => onopen?.(e.taskPath)}>
            <span class="nrow__kind">{e.kind}</span>
            <time class="nrow__time" datetime={e.at}>{e.at}</time>
            <span class="nrow__title">
              {titleOf(e.taskPath)}
              {#if oos}<span class="nrow__tag" data-testid="oos-tag">out of scope</span>{/if}
            </span>
            <span class="nrow__text">{e.text}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .nlist__head { display: flex; justify-content: space-between; align-items: center; padding: 6px 0; font-size: var(--fs-12); color: var(--label-2); }
  .nlist__mark { border: 0; background: none; color: var(--tint); cursor: pointer; font-size: var(--fs-12); }
  .nlist__mark:disabled { color: var(--label-3); cursor: default; }
  .nlist__empty { color: var(--label-3); font-size: var(--fs-12); }
  .nlist__rows { list-style: none; margin: 0; padding: 0; }
  .nrow { display: grid; grid-template-columns: auto 1fr; gap: 2px 8px; width: 100%; text-align: left; padding: 8px; border: 0; border-left: 3px solid transparent; background: none; color: var(--label-2); font-size: var(--fs-12); cursor: pointer; }
  .nrow--unread { color: inherit; font-weight: 600; }
  .nrow--urgent { border-left-color: var(--orange); background: var(--fill); }
  .nrow--failed { border-left-color: var(--red); }
  .nrow__kind { text-transform: uppercase; letter-spacing: 0.04em; }
  .nrow--urgent .nrow__kind { color: var(--orange); }
  .nrow--failed .nrow__kind { color: var(--red); }
  .nrow__time { color: var(--label-3); }
  .nrow--oos { border-left-color: var(--label-3); }
  .nrow__title { grid-column: 1 / -1; }
  .nrow__tag { margin-left: 6px; padding: 0 4px; border-radius: 3px; background: var(--fill); color: var(--label-3); font-size: var(--fs-12); font-weight: 400; text-transform: uppercase; letter-spacing: 0.04em; }
  .nrow__text { grid-column: 1 / -1; font-weight: 400; color: var(--label-2); }
</style>
