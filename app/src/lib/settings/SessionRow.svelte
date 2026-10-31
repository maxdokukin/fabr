<script lang="ts">
  import type { CcSession } from '../api/types';
  import { ago, sessionTitle, taskLine } from '../fabric/cc';
  import Button from '../ui/Button.svelte';
  import Row from '../ui/Row.svelte';

  let { s, busy = false, onopen, ontranscript, ontrack }: { s: CcSession; busy?: boolean; onopen: () => void; ontranscript: () => void; ontrack: () => void } = $props();
</script>

<Row label={sessionTitle(s)} sub={s.task ? taskLine(s.task) : `${ago(s.modified_at)} · ${s.messages} messages`}>
  <Button variant="tint" data-testid="cc-open-{s.id}" onclick={onopen}>Open</Button>
  {#if !s.task}<Button variant="plain" data-testid="cc-track-{s.id}" disabled={busy} onclick={ontrack}>Track as task</Button>{/if}
  <Button variant="plain" data-testid="cc-transcript-{s.id}" onclick={ontranscript}>Transcript</Button>
</Row>
