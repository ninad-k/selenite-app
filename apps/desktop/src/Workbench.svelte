<script>
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import TreeNode from './TreeNode.svelte';
  import Graph from './Graph.svelte';
  import Dialog from './Dialog.svelte';
  import Rename from './Rename.svelte';
  import DatabaseViews from './DatabaseViews.svelte';
  import Services from './Services.svelte';
  import AskVault from './AskVault.svelte';
  import EmbedDialog from './EmbedDialog.svelte';
  import { dirtyConfigurations, serviceDefaults } from './lib/services.js';
  import { buildTree } from './lib/tree.js';

  let vault = $state(null);
  let page = $state(null);
  let draft = $state('');
  let previewHtml = $state('');
  let mode = $state('preview');
  let screen = $state('page');
  let EditorComponent = $state(null);
  let busy = $state(false);
  let initialLoading = $state(true);
  let pendingPath = $state(null);
  let error = $state('');
  let notice = $state('');
  let pendingAction = $state(null);
  let modal = $state(null);
  let newPath = $state('');
  let newSource = $state('');
  let query = $state('');
  let results = $state([]);
  let searched = $state(false);
  let searching = $state(false);
  let links = $state({ backlinks: [], outgoing: [], graph: { nodes: [], edges: [] } });
  let linksLoaded = $state(false);
  let serviceDrafts = $state({});
  let embed = $state(null);
  let navigation = 0;
  let modeIntent = 0;
  let searchRequest = 0;
  const readyVaults = new Set();
  const tree = $derived(vault ? buildTree(vault.files) : []);
  const isToday = $derived(vault && page?.path === vault.today);
  const dirty = $derived(page !== null && draft !== page.source);
  const hasUnsaved = $derived(dirty || dirtyConfigurations(serviceDrafts).length > 0);
  const folder = $derived(page?.path.split('/').slice(0, -1).join(' / ') || 'Vault');

  function messageFor(reason) { return typeof reason === 'string' ? reason : reason?.message || String(reason); }
  function acceptPage(next) {
    modeIntent += 1;
    page = next;
    draft = next.source;
    previewHtml = next.html;
    mode = 'preview';
    screen = 'page';
    linksLoaded = false;
    links = { backlinks: [], outgoing: [], graph: { nodes: [], edges: [] } };
  }
  function requestNavigation(action) {
    if (busy) return;
    if (hasUnsaved) pendingAction = action;
    else void action();
  }
  async function continueNavigation(saveFirst) {
    const action = pendingAction;
    if (!action) return;
    if (saveFirst && !(await saveAll())) return;
    if (!saveFirst) {
      busy = true;
      try {
        const refreshed = await Promise.all(dirtyConfigurations(serviceDrafts).map(async ([service, config]) => {
          const current = await invoke('read_service_config', { service });
          const source = current.source ?? serviceDefaults[service];
          return [service, { ...config, expectedSource: current.source, source, original: source, dirty: false }];
        }));
        serviceDrafts = { ...serviceDrafts, ...Object.fromEntries(refreshed) };
      } catch (reason) { error = `Could not reload settings from disk. ${messageFor(reason)}`; return; }
      finally { busy = false; }
      draft = page.source;
    }
    pendingAction = null;
    await action();
  }
  async function announceReady(opened) {
    await tick();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    if (page?.path === opened.today && !readyVaults.has(opened.root)) {
      await invoke('frontend_ready', { path: page.path, fileCount: opened.files.length, title: page.title });
      readyVaults.add(opened.root);
    }
  }
  async function openVault(command) {
    busy = true; error = ''; notice = ''; pendingPath = null;
    const request = ++navigation;
    try {
      const opened = await invoke(command);
      if (opened && request === navigation) {
        const changedVault = opened.root !== vault?.root;
        vault = opened; acceptPage(opened.page);
        if (changedVault) { results = []; searched = false; query = ''; searchRequest += 1; serviceDrafts = {}; embed = null; }
        await announceReady(opened);
      }
    } catch (reason) { error = `Could not open the vault. ${messageFor(reason)}`; }
    finally { busy = false; initialLoading = false; }
  }
  async function loadPage(path) {
    error = ''; notice = ''; pendingPath = path;
    const request = ++navigation;
    try {
      const preview = await invoke('read_page', { path });
      if (request === navigation) acceptPage(preview);
    } catch (reason) { if (request === navigation) error = `Could not open this note. ${messageFor(reason)}`; }
    finally { if (request === navigation) pendingPath = null; }
  }
  function selectPage(path) {
    if (busy) return;
    if (path === page?.path) { navigation += 1; pendingPath = null; screen = 'page'; return; }
    requestNavigation(() => loadPage(path));
  }
  async function edit() {
    if (busy || pendingPath !== null) return;
    const intent = ++modeIntent;
    const path = page.path;
    const root = vault.root;
    try {
      if (!EditorComponent) EditorComponent = (await import('./Editor.svelte')).default;
      if (intent === modeIntent && page?.path === path && vault?.root === root && !busy && pendingPath === null) { mode = 'edit'; screen = 'page'; }
    } catch (reason) { error = `Could not load the editor. ${messageFor(reason)}`; }
  }
  async function preview() {
    if (busy || pendingPath !== null) return;
    const intent = ++modeIntent;
    const path = page.path;
    const source = draft;
    const root = vault.root;
    try {
      const html = dirty ? (await invoke('preview_markdown', { path, source })).html : page.html;
      if (intent === modeIntent && page?.path === path && vault?.root === root && draft === source) { previewHtml = html; mode = 'preview'; }
    } catch (reason) { error = `Could not preview this note. ${messageFor(reason)}`; }
  }
  async function save() {
    if (!page || busy || pendingPath !== null) return false;
    if (!dirty) return true;
    busy = true; error = ''; notice = '';
    try {
      const saved = await invoke('save_page', { path: page.path, expectedSource: page.source, source: draft });
      page = saved; previewHtml = saved.html; linksLoaded = false;
      notice = draft === saved.source ? 'Saved to Markdown' : 'Earlier draft saved; newer edits remain';
      return draft === saved.source;
    } catch (reason) { error = `Your edits are still here. Saving failed: ${messageFor(reason)}`; return false; }
    finally { busy = false; }
  }
  async function saveConfigurations() {
    for (const [service, config] of dirtyConfigurations(serviceDrafts)) {
      const saved = await invoke('save_service_config', { service, expectedSource: config.expectedSource, source: config.source });
      const current = serviceDrafts[service];
      serviceDrafts = { ...serviceDrafts, [service]: { ...current, expectedSource: saved.source, original: saved.source, dirty: current.source !== saved.source } };
    }
  }
  async function saveAll() {
    if (busy || pendingPath !== null) return false;
    if (dirty && !(await save())) return false;
    busy = true;
    try { await saveConfigurations(); return dirtyConfigurations(serviceDrafts).length === 0; }
    catch (reason) { error = `Settings were not saved. ${messageFor(reason)}`; return false; }
    finally { busy = false; }
  }
  async function saveSettings() {
    if (busy) throw new Error('A save is already in progress.');
    busy = true;
    try { await saveConfigurations(); }
    finally { busy = false; }
  }
  async function create() {
    busy = true; error = '';
    try {
      const created = await invoke('create_page', { path: newPath.trim(), source: newSource });
      vault = { ...vault, files: [...new Set([...vault.files, created.path])].sort() };
      acceptPage(created); modal = null; newPath = ''; newSource = ''; busy = false;
      await edit();
    } catch (reason) { error = `Could not create the note. ${messageFor(reason)}`; }
    finally { busy = false; }
  }
  async function refresh() {
    busy = true; error = '';
    try {
      const opened = await invoke('refresh_vault');
      vault = opened;
      if (page && opened.files.includes(page.path)) await loadPage(page.path);
      else acceptPage(opened.page);
      notice = 'Vault refreshed from disk';
    } catch (reason) { error = messageFor(reason); }
    finally { busy = false; }
  }
  async function refreshFiles() {
    const root = vault.root;
    try { const opened = await invoke('refresh_vault'); if (vault?.root === root) vault = opened; }
    catch (reason) { error = messageFor(reason); }
  }
  async function search() {
    const request = ++searchRequest;
    searching = true; error = '';
    try {
      const found = await invoke('search_pages', { query });
      if (request === searchRequest) { results = found; searched = true; }
    } catch (reason) { if (request === searchRequest) error = `Search failed. ${messageFor(reason)}`; }
    finally { if (request === searchRequest) searching = false; }
  }
  async function loadLinks(graph = false) {
    if (!page) return;
    const path = page.path;
    const root = vault.root;
    try {
      const found = await invoke('page_links', { path });
      if (page?.path === path && vault?.root === root) {
        links = found; linksLoaded = true;
        if (graph) screen = 'graph';
      }
    } catch (reason) { error = `Could not load links. ${messageFor(reason)}`; }
  }
  function databaseChanged(row) {
    vault = { ...vault, files: [...new Set([...vault.files, row.path])].sort() };
    if (page?.path === row.path && !dirty) {
      const root = vault.root;
      void invoke('read_page', { path: row.path }).then((next) => {
        if (vault?.root === root && page?.path === row.path && !dirty) { page = next; draft = next.source; previewHtml = next.html; linksLoaded = false; }
      }).catch((reason) => { error = messageFor(reason); });
    }
  }
  function renamed(opened) {
    vault = opened; acceptPage(opened.page); modal = null;
    notice = 'Note renamed; links updated';
  }
  function preventPreviewNavigation(node) {
    const prevent = (event) => {
      if (!(event.target instanceof Element)) return;
      const anchor = event.target.closest('a');
      if (!anchor) return;
      event.preventDefault();
      const href = anchor.getAttribute('href');
      if (event.type === 'click' && href?.startsWith('#page=')) {
        const params = new URLSearchParams(href.slice(1));
        const path = params.get('page');
        if (path) selectPage(path);
      }
    };
    node.addEventListener('click', prevent); node.addEventListener('auxclick', prevent);
    return { destroy() { node.removeEventListener('click', prevent); node.removeEventListener('auxclick', prevent); } };
  }
  function shortcuts(event) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      if (!document.querySelector('dialog[open]') && hasUnsaved) void saveAll();
    }
  }
  onMount(() => {
    void openVault('startup_vault');
    let unlisten;
    let destroyed = false;
    getCurrentWindow().onCloseRequested((event) => {
      if (document.querySelector('dialog[open]')) { event.preventDefault(); return; }
      if (hasUnsaved) { event.preventDefault(); pendingAction = () => getCurrentWindow().destroy(); }
    }).then((stop) => { if (destroyed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { destroyed = true; unlisten?.(); };
  });
</script>

<svelte:window onkeydown={shortcuts} />
<svelte:head><title>{page ? `${hasUnsaved ? '• ' : ''}${page.title} · Selenite` : 'Selenite'}</title></svelte:head>

<div class="app-shell" class:has-vault={vault !== null}>
  <aside class="sidebar" aria-label="Vault navigation">
    <div class="brand"><span class="brand-mark"><Icon name="crystal" size={26} /></span><span>Selenite</span><span class="brand-dot" title="Local workspace"></span></div>
    {#if vault}
      <div class="vault-heading"><span class="eyebrow">YOUR VAULT</span><h1 title={vault.root}>{vault.name}</h1></div>
      <div class="sidebar-actions">
        <button class="today-button" class:active={isToday && screen === 'page'} disabled={busy} onclick={() => requestNavigation(() => openVault('open_today'))}><Icon name="calendar" size={18} /><span>Today’s note</span><span class="today-shortcut">Journal</span></button>
        <div class="view-switcher">
          <button class:active={screen === 'search'} disabled={busy} onclick={() => (screen = 'search')}>Search</button>
          <button class:active={screen === 'graph'} disabled={busy} onclick={() => loadLinks(true)}>Graph</button>
          <button class:active={screen === 'databases'} disabled={busy} onclick={() => (screen = 'databases')}>Views</button>
        </div>
        <button class="today-button auxiliary-nav" class:active={screen === 'ask'} disabled={busy} onclick={() => (screen = 'ask')}>Ask your vault<span class="today-shortcut">With sources</span></button>
      </div>
      <div class="tree-heading"><span class="eyebrow">PAGES</span><span class="page-count">{vault.files.length}</span><button class="small-button" disabled={busy} onclick={() => requestNavigation(() => { error = ''; modal = 'new'; })} aria-label="Create a note">+</button></div>
      <nav class="file-tree" data-testid="file-tree" aria-label="Vault pages"><ul class="tree-root">{#each tree as node (node.path)}<TreeNode {node} selectedPath={pendingPath || page?.path} onselect={selectPage} disabled={busy} />{/each}</ul></nav>
      <div class="sidebar-footer">
        <button class="open-vault-button" disabled={busy} onclick={() => (screen = 'settings')}>Services & settings</button>
        <button class="open-vault-button" disabled={busy} onclick={() => requestNavigation(refresh)}>Refresh from disk</button>
        <button class="open-vault-button" disabled={busy} onclick={() => requestNavigation(() => openVault('choose_vault'))}><Icon name="folder-open" size={17} /><span>Open another vault</span></button>
        <div class="local-status"><span></span> Markdown on this device</div>
      </div>
    {:else}
      <div class="sidebar-intro"><span class="eyebrow">A LITTLE ROOM TO THINK</span><p>Keep your notes close.<br />Make space for what matters.</p></div>
      <div class="sidebar-footer welcome-footer"><Icon name="note" size={17} /><span>Plain Markdown. Yours to keep.</span></div>
    {/if}
  </aside>
  <main class="main-panel">
    <header class="topbar">
      <div class="breadcrumbs">{#if vault && page}<Icon name="folder" size={15} /><span>{folder}</span><span class="breadcrumb-divider">/</span><button class="breadcrumb-current text-button" onclick={() => (screen = 'page')}>{page.title}</button>{:else}<span class="eyebrow">YOUR LOCAL WORKSPACE</span>{/if}</div>
      {#if vault && page}<div class="editor-actions"><span class="save-state" role="status">{hasUnsaved ? 'Unsaved changes' : notice || 'Saved on disk'}</span><button class="small-button" disabled={busy || !hasUnsaved} onclick={saveAll}>Save <kbd>Ctrl S</kbd></button></div>{:else}<span class="preview-label"><span></span>Made for your thoughts</span>{/if}
    </header>
    {#if error}<div class="error-banner" role="alert" data-testid="app-error"><Icon name="warning" size={18} /><p>{error}</p><button class="icon-button" aria-label="Dismiss error" onclick={() => (error = '')}><Icon name="close" size={16} /></button></div>{/if}
    {#if vault && page}
      <div class="workspace-content" class:with-embed={embed !== null && screen === 'page'}>
      <div class="document-scroll" aria-busy={pendingPath !== null}>
        {#if screen === 'search'}
          <section class="search-panel"><span class="eyebrow">FIND A THOUGHT</span><h2>Search your vault</h2><form class="search-form" onsubmit={(event) => { event.preventDefault(); void search(); }}><input aria-label="Search query" type="search" placeholder='A word, "a phrase", or title:idea' bind:value={query} /><button class="small-button" disabled={searching || !query.trim()}>{searching ? 'Searching…' : 'Search'}</button></form>
          {#if searched}<p class="subtle">{results.length} matching pages</p>{/if}<div class="search-results">{#each results as result (result.path)}<button class="search-result" onclick={() => selectPage(result.path)}><strong>{result.title}</strong><span>{result.path}</span></button>{/each}</div>{#if searched && results.length === 0}<p class="empty-state">No pages matched. Try a different word or phrase.</p>{/if}</section>
        {:else if screen === 'graph'}<Graph graph={links.graph} selectedPath={page.path} onselect={selectPage} />
        {:else if screen === 'databases'}{#key vault.root}<DatabaseViews vaultKey={vault.root} onselect={selectPage} onchanged={databaseChanged} />{/key}
        {:else if screen === 'settings'}<Services bind:drafts={serviceDrafts} onsave={saveSettings} disabled={busy} onfileschanged={refreshFiles} />
        {:else if screen === 'ask'}<AskVault onselect={selectPage} onsettings={() => (screen = 'settings')} />
        {:else}
          <div class="document-meta"><span class="document-kind"><Icon name={isToday ? 'calendar' : 'note'} size={15} />{isToday ? 'TODAY’S NOTE' : 'MARKDOWN NOTE'}</span><div class="segmented"><button class:active={mode === 'preview'} disabled={busy || pendingPath !== null} onclick={preview}>Preview</button><button class:active={mode === 'edit'} disabled={busy || pendingPath !== null} onclick={edit}>Edit</button></div></div>
          <div class="document-title"><h2>{page.title}</h2><div class="note-path-row"><p>{page.path}</p><div class="page-tools"><button class="text-button" disabled={busy} onclick={() => (modal = 'embed')}>Embed…</button><button class="text-button" disabled={busy} onclick={() => requestNavigation(() => { modal = 'rename'; })}>Rename</button></div></div></div>
          {#if mode === 'edit' && EditorComponent}{#key `${vault.root}/${page.path}`}<EditorComponent value={draft} onchange={(value) => (draft = value)} onsave={save} disabled={busy || pendingPath !== null} />{/key}
          {:else}<article class="note-preview" data-testid="note-preview" data-page-path={page.path} aria-label={`${page.title} preview`}>{#if previewHtml.trim()}<div use:preventPreviewNavigation>{@html previewHtml}</div>{:else}<div class="empty-note"><Icon name="note" size={28} /><h3>A fresh page.</h3><p>Choose Edit to start writing.</p></div>{/if}</article>{/if}
          <section class="links-panel"><div class="section-heading"><h3>Page connections</h3><button class="text-button" onclick={() => loadLinks()}>{linksLoaded ? 'Refresh links' : 'Show links'}</button></div>{#if linksLoaded}<div class="link-columns"><div><span class="eyebrow">LINKED FROM</span>{#each links.backlinks as link}<button class="link-page" onclick={() => selectPage(link.path)}>{link.title}</button>{:else}<p class="subtle">No incoming links yet.</p>{/each}</div><div><span class="eyebrow">LINKS TO</span>{#each links.outgoing as link}<button class="link-page" onclick={() => selectPage(link.path)}>{link.title}{link.fragment ? ` · ${link.fragment}` : ''}</button>{:else}<p class="subtle">No resolved outgoing links.</p>{/each}</div></div>{/if}</section>
          <footer class="document-footer"><span class="small-rule"></span> A quiet place for your words.</footer>
        {/if}
      </div>
      {#if embed && screen === 'page'}<aside class="embed-pane" aria-label="Embedded resource"><div class="embed-toolbar"><span title={embed.url}>{embed.origin}</span><button class="small-button" onclick={() => (embed = null)}>Close embed</button></div><iframe data-testid="embed-frame" title={`Embedded resource from ${embed.origin}`} src={embed.url} sandbox="allow-scripts allow-same-origin allow-presentation" referrerpolicy="no-referrer" allow="fullscreen; picture-in-picture"></iframe></aside>{/if}
      </div>
    {:else}
      <div class="welcome-screen"><div class="welcome-emblem"><Icon name="crystal" size={48} /></div><span class="eyebrow welcome-eyebrow">WELCOME TO SELENITE</span><h1>Your thoughts,<br />at home.</h1><p class="welcome-description">Notes, daily pages, projects, and knowledge.<br />One folder of Markdown. A space you can shape.</p><button class="primary-button" disabled={busy} onclick={() => openVault('choose_vault')}><Icon name="folder-open" size={18} /><span>{initialLoading ? 'Opening your workspace…' : busy ? 'Opening vault…' : 'Open a vault'}</span>{#if !busy}<Icon name="arrow" size={17} />{/if}</button><p class="welcome-hint">Choose any folder of Markdown files.<br />Today’s note will be waiting in Journal.</p><div class="welcome-bottom"><span></span> Local files. No account needed. <span></span></div></div>
    {/if}
  </main>
</div>

{#if pendingAction}
  <Dialog label="Unsaved changes" disabled={busy} oncancel={() => (pendingAction = null)}><span class="eyebrow">YOUR WORDS MATTER</span><h2>Save your changes?</h2><p>You have unsaved {dirty ? `edits in ${page.title}` : 'service settings'}{dirty && dirtyConfigurations(serviceDrafts).length ? ' and service settings' : ''}. Save them before continuing, or discard the changes.</p>{#if error}<p class="dialog-error" role="alert">{error}</p>{/if}<div class="dialog-actions"><button class="small-button" disabled={busy} onclick={() => (pendingAction = null)}>Keep editing</button><button class="small-button" disabled={busy} onclick={() => continueNavigation(false)}>Discard changes</button><button class="primary-button" disabled={busy} onclick={() => continueNavigation(true)}>Save and continue</button></div></Dialog>
{/if}
{#if modal === 'new'}
  <Dialog label="Create a note" disabled={busy} oncancel={() => (modal = null)}><form onsubmit={(event) => { event.preventDefault(); void create(); }}><span class="eyebrow">MAKE A LITTLE SPACE</span><h2>Create a note</h2>{#if error}<p class="dialog-error" role="alert">{error}</p>{/if}<label class="field-label">Path inside your vault<input bind:value={newPath} placeholder="Projects/A new idea.md" required /></label><label class="field-label">Starting Markdown<textarea bind:value={newSource} rows="5" placeholder="# A new idea"></textarea></label><p class="subtle">Folders are created when needed. Existing notes are never replaced.</p><div class="dialog-actions"><button type="button" class="small-button" disabled={busy} onclick={() => (modal = null)}>Cancel</button><button class="primary-button" disabled={busy || !newPath.trim()}>Create note</button></div></form></Dialog>
{/if}
{#if modal === 'rename'}<Rename path={page.path} oncancel={() => (modal = null)} onrenamed={renamed} />{/if}
{#if modal === 'embed'}<EmbedDialog oncancel={() => (modal = null)} onload={(resource) => { embed = resource; modal = null; }} />{/if}
