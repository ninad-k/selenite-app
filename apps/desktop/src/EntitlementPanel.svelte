<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  let status = $state(null);
  let document = $state('');
  let busy = $state(false);
  let error = $state('');
  async function refresh() {
    busy = true; error = '';
    try { status = await invoke('pro_status'); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function install() {
    busy = true; error = '';
    try { status = await invoke('install_entitlement', { document }); document = ''; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  onMount(() => { void refresh(); });
</script>

<section class="service-tool-panel" data-testid="entitlement-panel">
  <div class="section-heading"><h3>Optional service entitlement</h3><button class="text-button" disabled={busy} onclick={refresh}>Refresh status</button></div>
  <p class="subtle">Community editing and local Markdown remain available regardless of entitlement status. Capabilities come from a signed document issued by your configured deployment.</p>
  {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if status}<div class="entitlement-status"><strong>{status.active ? 'Active entitlement' : 'Community access'}</strong><p>{status.message}</p>{#if status.subject}<p>Subject: {status.subject}</p>{/if}{#if status.expiresAt}<p>Expires: {new Date(status.expiresAt * 1000).toLocaleString()}</p>{/if}{#if status.capabilities.length}<p>Capabilities: {status.capabilities.join(', ')}</p>{/if}</div>{/if}
  <form onsubmit={(event) => { event.preventDefault(); void install(); }}><label class="field-label">Signed entitlement · JSON<textarea class="config-editor" rows="5" bind:value={document} placeholder={JSON.stringify({ key_id: '…', payload: '…', signature: '…' })} spellcheck="false" required></textarea></label><button class="small-button" disabled={busy || !document.trim()}>Verify and install entitlement</button></form>
</section>
