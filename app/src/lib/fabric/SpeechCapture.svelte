<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { isTauri } from '../api/env';
  import Button from '../ui/Button.svelte';

  let { path, log }: { path: string; log: string } = $props();

  let status = $state('');
  let playing = $state(false);
  const spoken = $derived(log.includes('cartesia_speak') && log.includes('succeeded'));

  async function save(bytes: Uint8Array) {
    if (!isTauri()) {
      status = 'Capture runs in the desktop app.';
      return;
    }
    let binary = '';
    for (const b of bytes) binary += String.fromCharCode(b);
    await invoke('call', { name: 'fabric_write_capture', req: { task_path: path, audio_base64: btoa(binary) } });
    status = 'Saved capture.wav in this node. Typed entry still works. Transcription runs only after you approve cartesia_transcribe.';
  }

  async function onFile(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    status = '';
    try {
      await save(new Uint8Array(await file.arrayBuffer()));
    } catch (e) {
      status = (e as Error).message || 'Could not save the recording.';
    }
  }

  async function record() {
    status = '';
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const rec = new MediaRecorder(stream);
      const chunks: Blob[] = [];
      rec.ondataavailable = (ev) => { if (ev.data.size) chunks.push(ev.data); };
      const done = new Promise<Blob>((resolve) => { rec.onstop = () => resolve(new Blob(chunks, { type: rec.mimeType })); });
      rec.start();
      status = 'Recording. Stop when you are done.';
      await new Promise((r) => setTimeout(r, 4000));
      rec.stop();
      stream.getTracks().forEach((t) => t.stop());
      const blob = await done;
      await save(new Uint8Array(await blob.arrayBuffer()));
    } catch (e) {
      status = (e as Error).message || 'Microphone capture is unavailable.';
    }
  }

  async function play() {
    if (!spoken || playing) return;
    playing = true;
    status = '';
    try {
      const res = await invoke<{ audio_base64: string }>('call', { name: 'fabric_read_speech', req: { task_path: path } });
      const bin = atob(res.audio_base64);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      const url = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }));
      const audio = new Audio(url);
      audio.onended = () => URL.revokeObjectURL(url);
      await audio.play();
    } catch (e) {
      status = (e as Error).message || 'speech.wav is not ready.';
    } finally {
      playing = false;
    }
  }
</script>

<section class="speech" data-testid="speech-capture">
  <h4>Speech</h4>
  <p class="dim">Record or pick audio. It is saved as capture.wav in this node. Playback of speech.wav waits until an approved cartesia_speak run succeeds.</p>
  <div class="row">
    <Button onclick={record}>Record 4s</Button>
    <label class="pick">Pick audio<input type="file" accept="audio/*" onchange={onFile} /></label>
    <Button disabled={!spoken || playing} onclick={play}>Play speech</Button>
  </div>
  {#if status}<p class="dim" role="status">{status}</p>{/if}
</section>

<style>
  .speech { display: flex; flex-direction: column; gap: 8px; padding: 10px 16px; }
  h4 { margin: 0; font-size: var(--fs-12); color: var(--label-2); text-transform: uppercase; }
  .dim { margin: 0; color: var(--label-2); font-size: var(--fs-12); }
  .row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .pick { font-size: var(--fs-12); }
  .pick input { display: block; margin-top: 4px; }
</style>
