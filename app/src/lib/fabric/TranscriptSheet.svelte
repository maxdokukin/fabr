<script lang="ts">
  import { createApi } from '../api/index';
  import { errText, type Transcript } from '../api/types';
  import Sheet from '../ui/Sheet.svelte';
  import { sessionTitle } from './cc';
  import TranscriptView from './TranscriptView.svelte';

  let { folder, id, title = '', onclose }: { folder: string; id: string; title?: string; onclose: () => void } = $props();

  let t = $state<Transcript | null>(null);
  let err = $state<string | null>(null);

  $effect(() => {
    const f = folder, i = id;
    t = null;
    err = null;
    createApi().then((a) => a.fabricCcTranscript(f, i)).then((r) => { t = r; }, (e) => { err = errText(e); });
  });
</script>

<Sheet open={true} title={sessionTitle(t ?? { title })} width={720} {onclose}>
  {#if err}<p class="hint">{err}</p>{:else if t}<TranscriptView transcript={t} />{:else}<p class="hint">Loading…</p>{/if}
</Sheet>
