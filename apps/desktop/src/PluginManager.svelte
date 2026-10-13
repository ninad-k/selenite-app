<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  let installed = $state([]);
  let catalog = $state(null);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let selected = $state(null);
  let review = $state(null);
  let input = $state('{}');
  let output = $state(null);
  let acknowledged = $state(false);

  async function refresh() {
    busy = true; error = '';
    try { installed = await invoke('list_installed_plugins'); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function fetchCatalog() {
    busy = true; error = ''; notice = '';
    try { catalog = await invoke('plugin_catalog'); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function install(id) {
    busy = true; error = ''; notice = '';
    try { await invoke('install_plugin', { id }); await refresh(); notice = `Installed ${id}. No capabilities were granted.`; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function run() {
    busy = true; error = ''; output = null;
    try { output = await invoke('run_plugin', { id: selected.id, input: JSON.parse(input), expectedPlugin: review.plugin, expectedConfig: review.configSource }); selected = null; review = null; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function beginRun(id) {
    busy = true; error = ''; input = '{}'; acknowledged = false; review = null;
    try { review = await invoke('prepare_plugin_run', { id }); selected = review.plugin; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  onMount(() => { void refresh(); });
</script>

<section class="service-tool-panel" data-testid="plugin-manager">
  <div class="section-heading"><h3>Installed plugins</h3><button class="text-button" disabled={busy} onclick={refresh}>Refresh</button></div>
  {#if error && !selected}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="subtle" role="status">{notice}</p>{/if}
  <div class="plugin-list">{#each installed as plugin (plugin.id)}<div class="plugin-item"><div><strong>{plugin.name} <span>{plugin.version}</span></strong><p>Requests: {plugin.capabilities.join(', ') || 'no vault access'}</p></div><button class="small-button" disabled={busy} onclick={() => beginRun(plugin.id)}>Run…</button></div>{:else}<p class="subtle">No plugins installed. Configure a signed catalog and its trusted public key to browse available plugins.</p>{/each}</div>
  <div class="catalog-heading"><h3>Configured marketplace</h3><button class="small-button" disabled={busy} onclick={fetchCatalog}>Fetch signed catalog</button></div>
  {#if catalog}<p class="subtle">Catalog entries are verified against the public keys in your configuration. Installing grants no access to your vault.</p><div class="plugin-list">{#each catalog.entries as entry (entry.manifest.id)}<div class="plugin-item"><div><strong>{entry.manifest.name} <span>{entry.manifest.version}</span></strong><p>Requests: {entry.manifest.capabilities.join(', ') || 'no vault access'}</p><code title={entry.sha256}>{entry.sha256.slice(0, 16)}…</code></div><button class="small-button" disabled={busy} onclick={() => install(entry.manifest.id)}>Install</button></div>{/each}</div>{/if}
  {#if output}<details class="plugin-output" open><summary>Plugin output · {output.host_calls} host calls · {output.fuel_consumed} fuel</summary><pre>{JSON.stringify(output.output, null, 2)}</pre></details>{/if}
</section>

{#if selected}
  <Dialog label={`Run ${selected.name}`} disabled={busy} oncancel={() => (selected = null)}>
    <span class="eyebrow">LOCAL WASM PLUGIN</span><h2>Run {selected.name}?</h2><p>Version <strong>{selected.version}</strong><br />SHA-256: <code class="plugin-digest">{selected.sha256}</code></p><p>Requested capabilities: {selected.capabilities.join(', ') || 'none'}<br /><strong>Granted for this run: {review.effectiveGrants.join(', ') || 'none'}</strong></p>
    {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
    <label class="field-label">Input · JSON<textarea class="config-editor" rows="6" bind:value={input} disabled={busy}></textarea></label>
    <label class="checkbox-label"><input type="checkbox" bind:checked={acknowledged} />Run this plugin with its configured grants.</label>
    <div class="dialog-actions"><button class="small-button" disabled={busy} onclick={() => (selected = null)}>Cancel</button><button class="primary-button" disabled={busy || !acknowledged} onclick={run}>Run plugin</button></div>
  </Dialog>
{/if}
