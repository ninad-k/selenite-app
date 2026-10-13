<script>
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  let { onfileschanged } = $props();
  let plan = $state(null);
  let report = $state(null);
  let busy = $state(false);
  let error = $state('');
  let acknowledged = $state(false);
  let resolving = $state(null);
  let mergedSource = $state('');
  let localSource = $state('');
  let resolveAcknowledged = $state(false);
  const remoteVersions = $derived(resolving && plan ? plan.snapshot.remote.filter((version) => version.path === resolving && plan.actions.find((action) => action.path === resolving)?.revisions.includes(version.revision)) : []);
  async function review() {
    busy = true; error = ''; plan = null; report = null; acknowledged = false;
    try { plan = await invoke('sync_plan'); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function run() {
    busy = true; error = ''; report = null;
    try { report = await invoke('sync_run', { plan }); plan = null; acknowledged = false; onfileschanged?.(); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function beginResolve(path) {
    busy = true; error = ''; resolveAcknowledged = false;
    try { const current = await invoke('read_sync_source', { path }); localSource = current.source ?? ''; mergedSource = localSource; resolving = path; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function resolve() {
    busy = true; error = '';
    try { report = await invoke('sync_resolve', { plan, path: resolving, source: mergedSource }); resolving = null; plan = null; onfileschanged?.(); }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
</script>

<section class="service-tool-panel" data-testid="sync-panel">
  <div class="section-heading"><h3>Review synchronization</h3><button class="small-button" disabled={busy} onclick={review}>{busy ? 'Working…' : 'Review remote changes'}</button></div>
  <p class="subtle">Remote storage is a Selenite version archive. It preserves versions without deleting or overwriting remote notes. Conflicting versions become Markdown copies in your local Conflicts folder.</p>
  {#if error && !resolving}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if plan}
    <p class="subtle">{plan.provider} · {plan.unchanged} unchanged · {plan.includeMetadata ? 'Vault metadata included' : 'Vault metadata excluded'}</p>
    <div class="sync-actions">{#each plan.actions as action}<div><span class={`action-kind ${action.kind}`}>{action.kind}</span><span>{action.path}</span>{#if action.kind === 'conflict'}<button class="text-button" disabled={busy} onclick={() => beginResolve(action.path)}>Resolve…</button>{/if}</div>{:else}<p class="subtle">No changes to apply.</p>{/each}</div>
    {#if plan.issues.length}<details class="sync-issues" open><summary>{plan.issues.length} items need attention</summary>{#each plan.issues as issue}<p><strong>{issue.path}</strong><br />{issue.reason}</p>{/each}</details>{/if}
    {#if plan.actions.length}<label class="checkbox-label"><input type="checkbox" bind:checked={acknowledged} />Apply these reviewed actions to my configured remote archive and vault.</label><button class="small-button" disabled={busy || !acknowledged} onclick={run}>Apply reviewed sync</button>{/if}
  {/if}
  {#if report}
    <p class="subtle" role="status">Uploaded {report.uploaded} · downloaded {report.downloaded} · unchanged {report.unchanged}</p>
    {#if report.resolved?.length}<p class="subtle">Published merged revisions:</p><ul class="sync-result-list">{#each report.resolved as path}<li>{path}</li>{/each}</ul>{/if}
    {#if report.conflicts.length}<p class="subtle">Preserved conflicting versions:</p><ul class="sync-result-list">{#each report.conflicts as path}<li>{path}</li>{/each}</ul>{/if}
    {#each report.issues as issue}<p class="dialog-error">{issue.path}: {issue.reason}</p>{/each}
  {/if}
</section>

{#if resolving}
  <Dialog label="Resolve a sync conflict" disabled={busy} oncancel={() => (resolving = null)}>
    <span class="eyebrow">PRESERVE AND RECONCILE</span><h2>Resolve this conflict</h2><p>{resolving}</p>
    <p>Compare your local content with the reviewed remote versions. Publishing preserves each original in Conflicts before applying your merged revision.</p>
    {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
    <details class="reviewed-config"><summary>Current local version</summary><pre>{localSource}</pre><button class="text-button" disabled={busy} onclick={() => (mergedSource = localSource)}>Replace editor with local version</button></details>
    {#each remoteVersions as version (version.revision)}<details class="reviewed-config"><summary>Remote version {version.revision.slice(0, 12)}</summary><pre>{version.source}</pre><button class="text-button" disabled={busy} onclick={() => (mergedSource = version.source)}>Replace editor with this version</button></details>{/each}
    <label class="field-label">Merged source<textarea class="config-editor" rows="12" bind:value={mergedSource} disabled={busy}></textarea></label>
    <label class="checkbox-label"><input type="checkbox" bind:checked={resolveAcknowledged} />Publish this merged revision over the reviewed versions, retaining their history.</label>
    <div class="dialog-actions"><button class="small-button" disabled={busy} onclick={() => (resolving = null)}>Cancel</button><button class="primary-button" disabled={busy || !resolveAcknowledged} onclick={resolve}>Publish merged revision</button></div>
  </Dialog>
{/if}
