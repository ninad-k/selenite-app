<script>
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  let { path, oncancel, onrenamed } = $props();
  let destination = $state('');
  let plan = $state(null);
  let busy = $state(false);
  let error = $state('');
  let acknowledged = $state(false);

  async function preview() {
    busy = true; error = ''; plan = null; acknowledged = false;
    try { plan = await invoke('rename_preview', { from: path, to: destination.trim() }); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function apply() {
    busy = true; error = '';
    try {
      const opened = await invoke('apply_rename', { from: plan.from, to: plan.to, expectedSources: plan.expectedSources, expectedChanges: plan.changes });
      onrenamed(opened);
    } catch (reason) { error = `Rename was not completed. ${String(reason)}`; }
    finally { busy = false; }
  }
</script>

<Dialog label="Rename note" {oncancel} disabled={busy}>
  <span class="eyebrow">KEEP YOUR CONNECTIONS</span><h2>Rename this note</h2><p class="subtle">{path}</p>
  {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
  <form onsubmit={(event) => { event.preventDefault(); void preview(); }}>
    <label class="field-label">New path<input required bind:value={destination} oninput={() => { plan = null; acknowledged = false; }} placeholder="Projects/New name.md" /></label>
    <button class="small-button" disabled={busy || !destination.trim()}>Review changes</button>
  </form>
  {#if plan}
    <p>Move to <strong>{plan.to}</strong> and update links in {plan.changes.length} page{plan.changes.length === 1 ? '' : 's'}.</p>
    <ul class="rename-changes">{#each plan.changes as change (change.path)}<li><details><summary>{change.path}</summary><pre>{change.updatedSource}</pre></details></li>{/each}</ul>
    <label class="checkbox-label"><input type="checkbox" bind:checked={acknowledged} />I reviewed the move and link changes.</label>
  {/if}
  <div class="dialog-actions"><button class="small-button" disabled={busy} onclick={oncancel}>Cancel</button><button class="primary-button" disabled={busy || !plan || !acknowledged} onclick={apply}>Rename and update links</button></div>
</Dialog>
