<script>
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  let { oncancel, onsaved } = $props();
  let kind = $state('schema');
  let id = $state('notes');
  let source = $state('');
  let expectedSource = $state(null);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state('');
  function sample() {
    const quoted = JSON.stringify(id.trim());
    return kind === 'schema'
      ? `id = ${quoted}\nname = "My notes"\n\n[source]\nkind = "folder"\npath = "Notes"\n\n[[fields]]\nname = "status"\ntype = "select"\noptions = ["idea", "active", "complete"]\n\n[[fields]]\nname = "due"\ntype = "date"\n`
      : `id = ${quoted}\nname = "All notes"\ndatabase = "notes"\nlayout = "table"\n# Other layouts: board, calendar, gallery.\n# group_by = "status"\n# calendar_field = "due"\n\n[[sort]]\nfield = "$title"\n`;
  }
  async function load() {
    busy = true; error = '';
    try { const definition = await invoke('read_definition', { kind, id: id.trim() }); expectedSource = definition.source; source = definition.source ?? sample(); loaded = true; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function save() {
    busy = true; error = '';
    try { await invoke('save_definition', { kind, id: id.trim(), expectedSource, source }); onsaved(); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
</script>

<Dialog label="Database and view definitions" disabled={busy} {oncancel}>
  <span class="eyebrow">ONE ENGINE, YOUR STRUCTURE</span><h2>{loaded ? `Edit ${kind}` : 'Database & view setup'}</h2>
  <p>A schema gives properties their types. A saved view chooses its database, filters, ordering, and layout. Definitions stay in your vault.</p>
  {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if !loaded}<form onsubmit={(event) => { event.preventDefault(); void load(); }}><label class="field-label">Definition<select bind:value={kind}><option value="schema">Database schema</option><option value="view">Saved view</option></select></label><label class="field-label">ID<input bind:value={id} required placeholder="notes" /></label><button class="small-button" disabled={busy || !id.trim()}>Load or create definition</button></form>
  {:else}<p class="subtle">.selenite/{kind === 'schema' ? 'schemas' : 'views'}/{id}.toml</p><label class="field-label">TOML definition<textarea class="config-editor" rows="16" spellcheck="false" bind:value={source} disabled={busy}></textarea></label>{/if}
  <div class="dialog-actions"><button class="small-button" disabled={busy} onclick={oncancel}>Cancel</button>{#if loaded}<button class="primary-button" disabled={busy} onclick={save}>Save definition</button>{/if}</div>
</Dialog>
