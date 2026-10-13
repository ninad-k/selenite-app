<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import Dialog from './Dialog.svelte';
  import DefinitionEditor from './DefinitionEditor.svelte';
  import { cellText, parseCell, calendarRows, monthDays } from './lib/cells.js';

  let { onselect, onchanged, vaultKey } = $props();
  let views = $state([]);
  let selected = $state('');
  let result = $state(null);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let setup = $state(false);
  let editing = $state(null);
  let values = $state({});
  let path = $state('');
  let body = $state('');
  let month = $state(`${new Date().getFullYear()}-${String(new Date().getMonth() + 1).padStart(2, '0')}`);
  let request = 0;
  const dates = $derived(result ? calendarRows(result.rows, result.view.calendar_field) : []);
  const datesByDay = $derived(new Map(dates));
  const days = $derived(monthDays(month));
  const rowsByPath = $derived(new Map((result?.rows || []).map((row) => [row.path, row])));

  async function loadList() {
    busy = true; error = '';
    try { views = await invoke('list_views'); if (views.length) { selected = views[0].id; await loadView(); } }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function installTemplates() {
    busy = true; error = ''; notice = '';
    try { const installed = await invoke('install_gtd_templates'); await loadList(); notice = `Installed ${installed.length} template definitions. Existing definitions were preserved.`; }
    catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  async function loadView() {
    if (!selected) return;
    const id = ++request; busy = true; error = '';
    try { const next = await invoke('query_view', { id: selected }); if (id === request) result = next; }
    catch (reason) { if (id === request) error = String(reason); }
    finally { if (id === request) busy = false; }
  }
  function editRow(row = null) {
    editing = row || 'new';
    values = Object.fromEntries(result.schema.fields.map((field) => {
      const cell = row?.cells[field.name];
      const value = field.type === 'checkbox' ? Boolean(cell?.value)
        : field.type === 'multi_select' && field.options?.length ? [...(cell?.value || [])]
        : field.type === 'multi_select' || field.type === 'relation' ? (cell?.value || []).join('\n')
        : cellText(cell);
      return [field.name, value];
    }));
    path = ''; body = ''; error = '';
  }
  async function saveRow() {
    busy = true; error = '';
    try {
      const cells = Object.fromEntries(result.schema.fields.map((field) => [field.name, parseCell(field, values[field.name])]));
      const args = editing === 'new'
        ? { database: result.schema.id, path: path.trim(), body, cells }
        : { database: result.schema.id, path: editing.path, expectedSource: editing.source, changes: cells };
      const row = await invoke(editing === 'new' ? 'create_database_row' : 'update_database_row', args);
      editing = null; onchanged?.(row); await loadView();
    } catch (reason) { error = String(reason); }
    finally { busy = false; }
  }
  onMount(() => { void vaultKey; void loadList(); });
</script>

<section class="database-panel" data-testid="database-views">
  <div class="section-heading"><div><span class="eyebrow">SHAPE YOUR WORKSPACE</span><h2>{result?.schema.name || 'Database views'}</h2></div><button class="small-button" disabled={busy} onclick={loadList}>Refresh</button></div>
  <div class="definition-actions"><button class="text-button" disabled={busy} onclick={() => (setup = true)}>Database & view setup</button><button class="text-button" disabled={busy} onclick={installTemplates}>Install GTD templates</button></div>
  {#if error && !editing}<p class="dialog-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="subtle" role="status">{notice}</p>{/if}
  {#if views.length}
    <div class="database-toolbar"><select aria-label="Saved view" bind:value={selected} onchange={loadView} disabled={busy}>{#each views as view (view.id)}<option value={view.id}>{view.name} · {view.layout}</option>{/each}</select><button class="small-button" disabled={busy || !result} onclick={() => editRow()}>New page</button></div>
  {:else if !busy}<div class="empty-state"><strong>Your Markdown can become a database.</strong><p>Create a schema and saved view with Database & view setup, or install the GTD templates for a ready-made starting point. Each row stays a regular note.</p></div>{/if}
  {#if result}
    <p class="subtle">{result.rows.length} pages · {result.view.name}{result.view.group_by ? ` · grouped by ${result.view.group_by}` : ''}</p>
    {#if result.view.layout === 'table'}
      <div class="database-table-scroll"><table class="database-table"><thead><tr><th>Page</th>{#each result.schema.fields as field}<th>{field.name}</th>{/each}<th><span class="sr-only">Actions</span></th></tr></thead><tbody>{#each result.rows as row (row.path)}<tr><td><button class="text-button" onclick={() => onselect(row.path)}>{row.title}</button></td>{#each result.schema.fields as field}<td>{row.cells[field.name]?.type === 'checkbox' ? (row.cells[field.name].value ? '✓' : '—') : cellText(row.cells[field.name])}</td>{/each}<td><button class="text-button" onclick={() => editRow(row)}>Edit</button></td></tr>{/each}</tbody></table></div>
    {:else if result.view.layout === 'board'}
      <div class="database-board">{#each result.groups as group}<section class="board-column"><h3>{cellText(group.key) || 'Unassigned'}<span>{group.rows.length}</span></h3>{#each group.rows as rowPath}{@const row = rowsByPath.get(rowPath)}{#if row}<div class="database-card"><button class="card-title" onclick={() => onselect(row.path)}>{row.title}</button>{#each result.schema.fields.filter((field) => field.name !== result.view.group_by).slice(0, 3) as field}{#if cellText(row.cells[field.name])}<p><span>{field.name}</span>{cellText(row.cells[field.name])}</p>{/if}{/each}<button class="text-button" onclick={() => editRow(row)}>Edit properties</button></div>{/if}{/each}</section>{/each}</div>
    {:else if result.view.layout === 'calendar'}
      <label class="field-label calendar-month">Month<input type="month" bind:value={month} /></label>
      <div class="calendar-weekdays">{#each ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as day}<span>{day}</span>{/each}</div>
      <div class="database-calendar">{#each days as date}<section class="calendar-day" class:outside-month={!date}>{#if date}<h3>{Number(date.slice(-2))}</h3>{#each datesByDay.get(date) || [] as row}<div class="calendar-row"><button class="text-button" onclick={() => onselect(row.path)} title={row.title}>{row.title}</button><button class="text-button" onclick={() => editRow(row)} aria-label={`Edit ${row.title}`}>⋯</button></div>{/each}{/if}</section>{/each}</div>
      {#if datesByDay.has('')}<details class="undated-rows"><summary>{datesByDay.get('').length} pages without a date</summary>{#each datesByDay.get('') as row}<div class="calendar-row"><button class="text-button" onclick={() => onselect(row.path)}>{row.title}</button><button class="text-button" onclick={() => editRow(row)}>Set a date</button></div>{/each}</details>{/if}
    {:else if result.view.layout === 'gallery'}
      <div class="database-gallery">{#each result.rows as row (row.path)}<div class="database-card gallery-card"><div class="gallery-cover">{cellText(row.cells[result.view.gallery_field]) || row.title.slice(0, 1).toUpperCase()}</div><button class="card-title" onclick={() => onselect(row.path)}>{row.title}</button>{#each result.schema.fields.slice(0, 4) as field}{#if cellText(row.cells[field.name])}<p><span>{field.name}</span>{cellText(row.cells[field.name])}</p>{/if}{/each}<button class="text-button" onclick={() => editRow(row)}>Edit properties</button></div>{/each}</div>
    {/if}
    {#if !result.rows.length}<p class="empty-state">No pages match this view yet.</p>{/if}
  {/if}
</section>

{#if editing}
  <Dialog label={editing === 'new' ? 'Create database page' : 'Edit page properties'} disabled={busy} oncancel={() => (editing = null)}>
    <span class="eyebrow">{result.schema.name}</span><h2>{editing === 'new' ? 'Create a page' : editing.title}</h2>
    {#if error}<p class="dialog-error" role="alert">{error}</p>{/if}
    <form onsubmit={(event) => { event.preventDefault(); void saveRow(); }}>
      {#if editing === 'new'}<label class="field-label">Vault-relative Markdown path<input bind:value={path} required placeholder={result.schema.source.kind === 'folder' ? `${result.schema.source.path}/New page.md` : 'New page.md'} /></label>{/if}
      {#each result.schema.fields as field}
        {#if field.type === 'checkbox'}<label class="checkbox-label"><input type="checkbox" bind:checked={values[field.name]} />{field.name}{field.required ? ' *' : ''}</label>
        {:else}<label class="field-label">{field.name}{field.required ? ' *' : ''}
          {#if field.type === 'select'}<select bind:value={values[field.name]} required={field.required}><option value="">Choose…</option>{#each field.options || [] as option}<option value={option}>{option}</option>{/each}</select>
          {:else if field.type === 'multi_select' && field.options?.length}<select multiple size={Math.min(field.options.length, 5)} bind:value={values[field.name]} required={field.required}>{#each field.options as option}<option value={option}>{option}</option>{/each}</select><span class="subtle">Hold Ctrl or Command to select more than one value.</span>
          {:else if field.type === 'multi_select' || field.type === 'relation'}<textarea rows="3" bind:value={values[field.name]} required={field.required} placeholder="One value per line"></textarea>
          {:else}<input type={field.type === 'number' ? 'number' : field.type === 'date' ? 'date' : field.type === 'url' ? 'url' : 'text'} step={field.type === 'number' ? 'any' : undefined} required={field.required} bind:value={values[field.name]} />{/if}
        </label>{/if}
      {/each}
      {#if editing === 'new'}<label class="field-label">Note body<textarea bind:value={body} rows="4"></textarea></label>{/if}
      <div class="dialog-actions"><button type="button" class="small-button" disabled={busy} onclick={() => (editing = null)}>Cancel</button><button class="primary-button" disabled={busy}>Save to Markdown</button></div>
    </form>
  </Dialog>
{/if}
{#if setup}<DefinitionEditor oncancel={() => (setup = false)} onsaved={() => { setup = false; void loadList(); }} />{/if}
