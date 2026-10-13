<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { serviceDefaults, updateConfiguration } from './lib/services.js';
  import PluginManager from './PluginManager.svelte';
  import SyncPanel from './SyncPanel.svelte';
  import EntitlementPanel from './EntitlementPanel.svelte';
  let { drafts = $bindable({}), onsave, disabled = false, onfileschanged } = $props();
  let service = $state('ai');
  let loading = $state(false);
  let saving = $state(false);
  let error = $state('');
  let notice = $state('');
  let destroyed = false;
  const names = { ai: 'AI provider', sync: 'Cloud sync', plugins: 'Plugins & marketplace', pro: 'Optional paid services' };
  async function load() {
    if (drafts[service]) return;
    const selected = service; loading = true; error = ''; notice = '';
    try {
      const config = await invoke('read_service_config', { service: selected });
      if (destroyed) return;
      const source = config.source ?? serviceDefaults[selected];
      drafts = { ...drafts, [selected]: { source, original: source, expectedSource: config.source, path: config.path, dirty: false } };
    } catch (reason) { error = String(reason); }
    finally { loading = false; }
  }
  async function save() {
    saving = true; error = ''; notice = '';
    try {
      if (drafts[service].expectedSource === null) drafts = { ...drafts, [service]: { ...drafts[service], dirty: true } };
      await onsave(); notice = 'Configuration saved on this device.';
    }
    catch (reason) { error = String(reason); }
    finally { saving = false; }
  }
  onMount(() => { void load(); return () => { destroyed = true; }; });
</script>

<section class="services-panel" data-testid="service-settings">
  <span class="eyebrow">YOUR CHOICES, YOUR VAULT</span><h2>Services & settings</h2>
  <p class="subtle">Services stay inactive until configured and explicitly used. Keep API keys in environment variables, never in notes or this configuration.</p>
  <div class="service-tabs">{#each Object.entries(names) as [id, name]}<button class="small-button" class:active={service === id} disabled={loading || saving || disabled} onclick={() => { service = id; void load(); }}>{name}</button>{/each}</div>
  {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="subtle" role="status">{notice}</p>{/if}
  {#if drafts[service]}
    <div class="service-description">
      <h3>{names[service]}</h3>
      {#if service === 'ai'}<p>Set a compatible provider URL and model names. <code>api_key_env</code> names an environment variable containing your key. Limit retrieval and embedding with vault-relative <code>folders</code>; an empty list includes the vault.</p>
      {:else if service === 'sync'}<p>Choose Google Drive or OneDrive, an existing archive folder, and the environment variable holding your access token. Review remote changes before applying them; conflicting versions are preserved as separate notes.</p>
      {:else if service === 'plugins'}<p>Configure plugin sources and permissions for your deployment. Only enable plugins you trust with the permissions you grant.</p>
      {:else}<p>Configure trusted public keys and optional issuer restrictions for signed entitlements. Capabilities are chosen by your deployment; community editing and local Markdown remain available.</p>{/if}
    </div>
    <label class="field-label">Advanced configuration · TOML<textarea class="config-editor" rows="14" spellcheck="false" disabled={saving || disabled} value={drafts[service].source} oninput={(event) => { drafts = { ...drafts, [service]: updateConfiguration(drafts[service], event.currentTarget.value) }; }}></textarea></label>
    <div class="service-save"><span class="subtle">{drafts[service].dirty ? 'Unsaved configuration' : drafts[service].expectedSource === null ? 'Suggested configuration · not saved yet' : 'Saved configuration'}</span><button class="small-button" disabled={saving || loading || disabled} onclick={save}>{saving ? 'Saving…' : 'Save configurations'}</button></div>
    <p class="subtle config-path">{drafts[service].path}</p>
    {#if service === 'plugins'}<PluginManager />{/if}
    {#if service === 'sync'}<SyncPanel {onfileschanged} />{/if}
    {#if service === 'pro'}<EntitlementPanel />{/if}
  {:else if loading}<p class="subtle">Reading local settings…</p>{/if}
</section>
