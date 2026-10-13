<script>
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import Icon from './Icon.svelte';
  import TreeNode from './TreeNode.svelte';
  import { buildTree } from './lib/tree.js';

  let vault = $state(null);
  let page = $state(null);
  let busy = $state(false);
  let initialLoading = $state(true);
  let pendingPath = $state(null);
  let error = $state('');
  let navigation = 0;
  const readyVaults = new Set();
  const tree = $derived(vault ? buildTree(vault.files) : []);
  const isToday = $derived(vault && page?.path === vault.today);
  const folder = $derived(page?.path.split('/').slice(0, -1).join(' / ') || 'Vault');

  function messageFor(reason) {
    return typeof reason === 'string' ? reason : reason?.message || String(reason);
  }

  async function openVault(command) {
    if (busy) return;
    busy = true;
    error = '';
    pendingPath = null;
    const request = ++navigation;
    try {
      const opened = await invoke(command);
      if (opened && request === navigation) {
        vault = opened;
        page = opened.page;
        await tick();
        await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        if (page.path === opened.today && !readyVaults.has(opened.root)) {
          await invoke('frontend_ready', {
            path: page.path,
            fileCount: opened.files.length,
            title: page.title,
          });
          readyVaults.add(opened.root);
        }
      }
    } catch (reason) {
      error = `Could not open the vault. ${messageFor(reason)}`;
    } finally {
      busy = false;
      initialLoading = false;
    }
  }

  async function selectPage(path) {
    if (busy) return;
    if (path === page?.path) {
      if (pendingPath !== null) {
        navigation += 1;
        pendingPath = null;
        error = '';
      }
      return;
    }
    error = '';
    pendingPath = path;
    const request = ++navigation;
    try {
      const preview = await invoke('read_page', { path });
      if (request === navigation) page = preview;
    } catch (reason) {
      if (request === navigation) error = `Could not open this note. ${messageFor(reason)}`;
    } finally {
      if (request === navigation) pendingPath = null;
    }
  }

  function preventPreviewNavigation(node) {
    function prevent(event) {
      if (event.target instanceof Element && event.target.closest('a')) event.preventDefault();
    }
    node.addEventListener('click', prevent);
    node.addEventListener('auxclick', prevent);
    return {
      destroy() {
        node.removeEventListener('click', prevent);
        node.removeEventListener('auxclick', prevent);
      },
    };
  }

  onMount(() => {
    void openVault('startup_vault');
  });
</script>

<svelte:head>
  <title>{page ? `${page.title} · Selenite` : 'Selenite'}</title>
</svelte:head>

<div class="app-shell" class:has-vault={vault !== null}>
  <aside class="sidebar" aria-label="Vault navigation">
    <div class="brand">
      <span class="brand-mark"><Icon name="crystal" size={26} /></span>
      <span>Selenite</span>
      <span class="brand-dot" title="Local workspace"></span>
    </div>

    {#if vault}
      <div class="vault-heading">
        <span class="eyebrow">YOUR VAULT</span>
        <h1 title={vault.root}>{vault.name}</h1>
      </div>
      <div class="sidebar-actions">
        <button class="today-button" class:active={isToday} disabled={busy} onclick={() => openVault('open_today')}>
          <Icon name="calendar" size={18} />
          <span>Today’s note</span>
          <span class="today-shortcut">Journal</span>
        </button>
      </div>
      <div class="tree-heading">
        <span class="eyebrow">PAGES</span>
        <span class="page-count">{vault.files.length}</span>
      </div>
      <nav class="file-tree" data-testid="file-tree" aria-label="Vault pages">
        <ul class="tree-root">
          {#each tree as node (node.path)}
            <TreeNode {node} selectedPath={pendingPath || page?.path} onselect={selectPage} disabled={busy} />
          {/each}
        </ul>
      </nav>
      <div class="sidebar-footer">
        <button class="open-vault-button" disabled={busy} onclick={() => openVault('choose_vault')}>
          <Icon name="folder-open" size={17} />
          <span>Open another vault</span>
        </button>
        <div class="local-status"><span></span> Stored on this device</div>
      </div>
    {:else}
      <div class="sidebar-intro">
        <span class="eyebrow">A LITTLE ROOM TO THINK</span>
        <p>Keep your notes close.<br />Make space for what matters.</p>
      </div>
      <div class="sidebar-footer welcome-footer">
        <Icon name="note" size={17} />
        <span>Plain Markdown. Yours to keep.</span>
      </div>
    {/if}
  </aside>

  <main class="main-panel">
    <header class="topbar">
      <div class="breadcrumbs">
        {#if vault && page}
          <Icon name="folder" size={15} />
          <span>{folder}</span>
          <span class="breadcrumb-divider">/</span>
          <span class="breadcrumb-current">{page.title}</span>
        {:else}
          <span class="eyebrow">YOUR LOCAL WORKSPACE</span>
        {/if}
      </div>
      <span class="preview-label"><span></span> {vault ? 'Read-only preview' : 'Made for your thoughts'}</span>
    </header>

    {#if error}
      <div class="error-banner" role="alert" data-testid="app-error">
        <Icon name="warning" size={18} />
        <p>{error}</p>
        <button class="icon-button" aria-label="Dismiss error" onclick={() => (error = '')}><Icon name="close" size={16} /></button>
      </div>
    {/if}

    {#if vault && page}
      <div class="document-scroll" aria-busy={pendingPath !== null}>
        <div class="document-meta">
          <span class="document-kind"><Icon name={isToday ? 'calendar' : 'note'} size={15} />{isToday ? 'TODAY’S NOTE' : 'MARKDOWN NOTE'}</span>
          <span class="document-status" role="status">
            {#if pendingPath}<span class="loading-dot"></span>Opening note…{:else}<Icon name="check" size={14} />{isToday && vault.created ? 'Created from your journal template' : 'Opened from disk'}{/if}
          </span>
        </div>
        <div class="document-title">
          <h2>{page.title}</h2>
          <p>{page.path}</p>
        </div>
        <article
          class="note-preview"
          data-testid="note-preview"
          data-page-path={page.path}
          aria-label={`${page.title} preview`}
        >
          {#if page.html.trim()}
            <!-- The Rust renderer sanitizes HTML; links stay inside this read-only preview. -->
            <div use:preventPreviewNavigation>{@html page.html}</div>
          {:else}
            <div class="empty-note"><Icon name="note" size={28} /><h3>A fresh page.</h3><p>This note is empty. Its Markdown file is ready for you.</p></div>
          {/if}
        </article>
        <footer class="document-footer"><span class="small-rule"></span> A quiet place for your words.</footer>
      </div>
    {:else}
      <div class="welcome-screen">
        <div class="welcome-emblem"><Icon name="crystal" size={48} /></div>
        <span class="eyebrow welcome-eyebrow">WELCOME TO SELENITE</span>
        <h1>Your thoughts,<br />at home.</h1>
        <p class="welcome-description">A simple space for your notes and daily pages.<br />Open a folder, and pick up where your mind left off.</p>
        <button class="primary-button" disabled={busy} onclick={() => openVault('choose_vault')}>
          <Icon name="folder-open" size={18} />
          <span>{initialLoading ? 'Opening your workspace…' : busy ? 'Opening vault…' : 'Open a vault'}</span>
          {#if !busy}<Icon name="arrow" size={17} />{/if}
        </button>
        <p class="welcome-hint">Choose any folder of Markdown files.<br />Today’s note will be waiting in Journal.</p>
        <div class="welcome-bottom"><span></span> Local files. No account needed. <span></span></div>
      </div>
    {/if}
  </main>
</div>
