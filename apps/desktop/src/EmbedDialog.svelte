<script>
  import Dialog from './Dialog.svelte';
  import { normalizeEmbedUrl } from './lib/embeds.js';
  let { oncancel, onload } = $props();
  let value = $state('');
  const candidate = $derived.by(() => {
    try { return { ...normalizeEmbedUrl(value), error: null }; }
    catch (reason) { return { error: value.trim() ? String(reason.message || reason) : null }; }
  });
</script>

<Dialog label="Open embedded content" {oncancel}>
  <span class="eyebrow">A RESOURCE BESIDE YOUR NOTES</span><h2>Open an embed</h2>
  <form onsubmit={(event) => { event.preventDefault(); if (candidate.url) onload({ url: candidate.url, origin: candidate.origin }); }}>
    <label class="field-label">HTTPS embed URL<input type="url" bind:value required placeholder="https://video.example/embed/course" /></label>
    {#if candidate.error}<p class="dialog-error" role="alert">{candidate.error}</p>{/if}
    {#if candidate.origin}<p>Loading connects to <strong>{candidate.origin}</strong>. That site will receive your connection and may use its existing cookies. The resource appears beside your note in a sandboxed frame.</p>{:else}<p>Paste an embed URL or a Google Drive file link. Content loads only after you select Load; note links never load embedded content automatically.</p>{/if}
    <p class="subtle">Some sites block embedding. Close the panel at any time to stop showing it.</p>
    <div class="dialog-actions"><button type="button" class="small-button" onclick={oncancel}>Cancel</button><button class="primary-button" disabled={!candidate.url}>Load resource</button></div>
  </form>
</Dialog>
