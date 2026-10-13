// Exercise the real Tauri WebView. All generated notes and evidence are retained.
// Usage: node scripts/workbench-smoke.mjs [target/release/selenite-desktop.exe]
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';

const root = realpathSync(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
function within(path) {
  const absolute = resolve(root, path);
  const part = relative(root, absolute);
  assert(part !== '..' && !part.startsWith(`..${sep}`) && !isAbsolute(part), 'Path must stay inside this repository');
  let existing = absolute;
  while (!existsSync(existing)) existing = dirname(existing);
  const canonical = relative(root, realpathSync(existing));
  assert(canonical !== '..' && !canonical.startsWith(`..${sep}`) && !isAbsolute(canonical), 'Existing ancestors must stay inside this repository');
  return absolute;
}
const executable = within(process.argv[2] ?? 'target/release/selenite-desktop.exe');
assert(existsSync(executable), 'Build the native executable before running this smoke test');
const output = within(join('.local', 'workbench-smoke', `${Date.now()}-${process.pid}`));
const vault = join(output, 'vault');
const temp = within(join('.local', 'tmp'));
mkdirSync(vault, { recursive: true });
mkdirSync(temp, { recursive: true });
const alphaInitial = '# Alpha\n\nA starting thought. [[Beta]]\n';
const betaInitial = '# Beta\n\nBack to [[Alpha]].\n';
writeFileSync(join(vault, 'Alpha.md'), alphaInitial, { flag: 'wx' });
writeFileSync(join(vault, 'Beta.md'), betaInitial, { flag: 'wx' });
const report = join(output, 'ready.json');
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const server = createServer();
await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
const port = server.address().port;
await new Promise((done) => server.close(done));
let remoteAttempts = 0;
const networkTrap = createServer((socket) => { remoteAttempts += 1; socket.destroy(); });
await new Promise((done, reject) => { networkTrap.once('error', reject); networkTrap.listen(0, '127.0.0.1', done); });
const disabledEndpoint = `http://127.0.0.1:${networkTrap.address().port}/`;
const launched = performance.now();
const child = spawn(executable, [], {
  cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  env: {
    ...process.env, TEMP: temp, TMP: temp,
    SELENITE_VAULT: vault, SELENITE_SMOKE_REPORT: report,
    SELENITE_WEBVIEW_DATA_DIR: join(output, 'webview-profile'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
  },
});
const logs = [];
child.stdout.on('data', (data) => logs.push(data.toString()));
child.stderr.on('data', (data) => logs.push(data.toString()));
let launchError;
child.once('error', (error) => { launchError = error; });
const closed = new Promise((done) => child.once('close', done));
const checks = [];
const runtimeErrors = [];
let client;
let ready;
let failure;

async function until(operation, label, timeout = 30000) {
  const start = performance.now();
  while (performance.now() - start < timeout) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`Native app exited ${child.exitCode}: ${logs.join('')}`);
    const value = await operation();
    if (value) return value;
    await delay(25);
  }
  throw new Error(`Timed out waiting for ${label}`);
}
async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((done, reject) => { socket.addEventListener('open', done, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  let sequence = 0;
  const requests = new Map();
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data);
    if (message.method === 'Runtime.exceptionThrown') runtimeErrors.push(message.params.exceptionDetails);
    const request = requests.get(message.id);
    if (!request) return;
    requests.delete(message.id); clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  return {
    socket,
    call(method, params = {}) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { requests.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 30000);
        requests.set(id, { resolve, reject, timer });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
  };
}
async function evaluate(expression, awaitPromise = false) {
  const response = await client.call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise });
  assert(!response.exceptionDetails, JSON.stringify(response.exceptionDetails));
  return response.result.value;
}
async function native(command, args = {}) {
  return evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})`, true);
}
async function clickText(text, selector = 'button') {
  await until(async () => evaluate(`(() => { const button = [...document.querySelectorAll(${JSON.stringify(selector)})].find(node => node.textContent.trim() === ${JSON.stringify(text)}); if (!button || button.disabled) return false; button.click(); return true; })()`), `button ${text}`);
}
async function setValue(selector, value) {
  const changed = await evaluate(`(() => { const node = document.querySelector(${JSON.stringify(selector)}); if (!node) return false; const descriptor = Object.getOwnPropertyDescriptor(node instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : node instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype, 'value'); descriptor.set.call(node, ${JSON.stringify(value)}); node.dispatchEvent(new Event('input', {bubbles:true})); node.dispatchEvent(new Event('change', {bubbles:true})); return true; })()`);
  assert(changed, `Missing input ${selector}`);
  await delay(40);
}
async function clickPage(path) {
  await until(async () => evaluate(`(() => { const file = [...document.querySelectorAll('.file-row')].find(node => node.title === ${JSON.stringify(path)}); if (!file || file.disabled) return false; file.click(); return true; })()`), `file ${path}`);
}
async function pageIs(path) {
  return until(() => evaluate(`document.querySelector('[data-testid="note-preview"]')?.dataset.pagePath === ${JSON.stringify(path)}`), `preview ${path}`);
}
async function enterEditor(source) {
  await clickText('Edit', '.segmented button');
  await until(() => evaluate(`!!document.querySelector('.cm-content')`), 'CodeMirror editor');
  await evaluate(`document.querySelector('.cm-content').focus()`);
  await client.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 2 });
  await client.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 2 });
  await client.call('Input.insertText', { text: source });
  await until(() => evaluate(`document.querySelector('.save-state')?.textContent.includes('Unsaved')`), 'unsaved editor draft');
}
async function screenshot(name) {
  const result = await client.call('Page.captureScreenshot', { format: 'png' });
  writeFileSync(join(output, `${name}.png`), Buffer.from(result.data, 'base64'), { flag: 'wx' });
}
async function check(name, action) {
  const started = performance.now();
  await action();
  const result = { name, elapsedMs: performance.now() - started, passed: true };
  checks.push(result); console.log(JSON.stringify(result));
}

try {
  ready = await until(() => {
    if (!existsSync(report)) return null;
    try { return JSON.parse(readFileSync(report, 'utf8')); } catch { return null; }
  }, 'native readiness');
  ready.processToReadyMs = performance.now() - launched;
  assert.equal(ready.processId, child.pid);
  const target = await until(async () => {
    try { return (await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json())).find((target) => target.type === 'page' && target.webSocketDebuggerUrl); }
    catch { return null; }
  }, 'native WebView debugging target');
  client = await connect(target.webSocketDebuggerUrl);
  await client.call('Runtime.enable');
  await check('native startup and existing preview hooks', async () => {
    await pageIs(ready.pagePath);
    assert(await evaluate(`!!document.querySelector('[data-testid="file-tree"]')`));
    assert(!(await evaluate(`!!document.querySelector('[data-testid="embed-frame"]')`)));
    assert(!(await evaluate(`document.querySelector('[data-testid="app-error"]')?.textContent`)));
    await screenshot('01-startup');
  });

  await check('dirty navigation keeps or discards a CodeMirror draft explicitly', async () => {
    await clickPage('Alpha.md'); await pageIs('Alpha.md');
    await enterEditor('# Alpha\n\nA draft that must not be saved automatically. [[Beta]]\n');
    await clickText('Preview', '.segmented button'); await pageIs('Alpha.md');
    assert(await evaluate(`document.querySelector('[data-testid="note-preview"]').textContent.includes('must not be saved automatically')`));
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), alphaInitial);
    await clickText('Edit', '.segmented button');
    await clickPage('Beta.md');
    await until(() => evaluate(`document.querySelector('dialog[open]')?.textContent.includes('Save your changes?')`), 'unsaved navigation dialog');
    await clickText('Keep editing', 'dialog button');
    assert(await evaluate(`document.querySelector('.cm-content')?.textContent.includes('must not be saved')`));
    await clickPage('Beta.md');
    await clickText('Discard changes', 'dialog button'); await pageIs('Beta.md');
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), alphaInitial);
  });

  const savedAlpha = '# Alpha\n\nSaved nativeeditorneedle from CodeMirror. [[Beta]]\n';
  await check('save Markdown and reopen the same source', async () => {
    await clickPage('Alpha.md'); await pageIs('Alpha.md'); await enterEditor(savedAlpha);
    await client.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 's', code: 'KeyS', windowsVirtualKeyCode: 83, modifiers: 2 });
    await client.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 's', code: 'KeyS', windowsVirtualKeyCode: 83, modifiers: 2 });
    await until(() => evaluate(`!document.querySelector('.save-state')?.textContent.includes('Unsaved')`), 'Ctrl+S save');
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), savedAlpha);
    await clickText('Preview', '.segmented button'); await pageIs('Alpha.md');
    assert(await evaluate(`document.querySelector('[data-testid="note-preview"]').textContent.includes('nativeeditorneedle')`));
    await clickPage('Beta.md'); await pageIs('Beta.md'); await clickPage('Alpha.md'); await pageIs('Alpha.md');
    assert.equal((await native('read_page', { path: 'Alpha.md' })).source, savedAlpha);
    await screenshot('02-saved-note');
  });

  await check('pending navigation locks editing and returning cancels stale reads', async () => {
    await clickText('Edit', '.segmented button');
    await until(() => evaluate(`!!document.querySelector('.cm-content')`), 'editor before delayed navigation');
    // Flush Svelte's microtasks before yielding to native IPC responses. The
    // immutable Tauri bridge remains intact; both clicks are real UI actions.
    const locked = await evaluate(`(async () => {
      [...document.querySelectorAll('.file-row')].find(node => node.title === 'Beta.md').click();
      await Promise.resolve(); await Promise.resolve();
      const result = { editor: document.querySelector('.cm-content')?.getAttribute('contenteditable'), buttons: [...document.querySelectorAll('.segmented button')].every(button => button.disabled) };
      [...document.querySelectorAll('.file-row')].find(node => node.title === 'Alpha.md').click();
      return result;
    })()`, true);
    assert.deepEqual(locked, { editor: 'false', buttons: true });
    await native('read_page', { path: 'Beta.md' });
    await delay(50);
    assert(await evaluate(`document.title.startsWith('Alpha') && document.querySelector('.cm-content')?.getAttribute('contenteditable') === 'true'`));
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), savedAlpha);
    await clickText('Preview', '.segmented button'); await pageIs('Alpha.md');
  });

  await check('embeds validate HTTPS and wait for explicit load without network access', async () => {
    await clickText('Embed…', '.page-tools button');
    await setValue('dialog input[type="url"]', 'http://example.invalid/course');
    assert(await evaluate(`document.querySelector('dialog [role="alert"]')?.textContent.includes('HTTPS')`));
    assert(await evaluate(`document.querySelector('dialog .primary-button').disabled`));
    await setValue('dialog input[type="url"]', 'https://example.invalid/course');
    assert(await evaluate(`document.querySelector('dialog').textContent.includes('https://example.invalid')`));
    assert(!(await evaluate(`!!document.querySelector('iframe')`)));
    await clickText('Cancel', 'dialog button');
    assert(!(await evaluate(`!!document.querySelector('iframe')`)));
  });

  await check('native close protects an unsaved draft', async () => {
    await enterEditor(`${savedAlpha}\nUnsaved close protection.\n`);
    if (process.platform === 'win32') {
      await new Promise((done, reject) => {
        const closer = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `$process = Get-Process -Id ${child.pid}; if (-not $process.CloseMainWindow()) { exit 1 }`], { cwd: root, windowsHide: true, stdio: 'ignore', env: { ...process.env, TEMP: temp, TMP: temp } });
        closer.once('error', reject); closer.once('close', (code) => code === 0 ? done() : reject(new Error(`CloseMainWindow failed: ${code}`)));
      });
      await until(() => evaluate(`document.querySelector('dialog[open]')?.textContent.includes('Save your changes?')`), 'native close confirmation');
      await clickText('Keep editing', 'dialog button');
      assert.equal(child.exitCode, null);
    }
    await clickPage('Beta.md'); await clickText('Discard changes', 'dialog button'); await pageIs('Beta.md');
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), savedAlpha);
  });

  const externalAlpha = '# Alpha\n\nExternal nativeeditorneedle stays intact. [[Beta]]\n';
  await check('external-edit conflict retains both disk content and the unsaved draft', async () => {
    await clickPage('Alpha.md'); await pageIs('Alpha.md');
    await enterEditor('# Alpha\n\nConflicting local draft must remain recoverable.\n');
    writeFileSync(join(vault, 'Alpha.md'), externalAlpha);
    await evaluate(`document.querySelector('.editor-actions button').click()`);
    await until(() => evaluate(`document.querySelector('[data-testid="app-error"]')?.textContent.includes('Saving failed')`), 'conflict notification');
    assert.equal(readFileSync(join(vault, 'Alpha.md'), 'utf8'), externalAlpha);
    assert(await evaluate(`document.querySelector('.cm-content').textContent.includes('Conflicting local draft')`));
    await screenshot('03-save-conflict');
    await clickPage('Beta.md'); await clickText('Discard changes', 'dialog button'); await pageIs('Beta.md');
    await clickPage('Alpha.md'); await pageIs('Alpha.md');
  });

  await check('full text search, backlinks, and graph show saved Markdown links', async () => {
    await clickText('Show links');
    await until(() => evaluate(`[...document.querySelectorAll('.link-page')].some(node => node.textContent === 'Beta')`), 'backlink and outgoing link');
    await clickText('Graph', '.view-switcher button');
    await until(() => evaluate(`document.querySelectorAll('.graph-node').length >= 3 && document.querySelectorAll('.vault-graph line').length >= 2`), 'graph edges');
    await screenshot('04-graph');
    await clickText('Search', '.view-switcher button');
    await setValue('input[aria-label="Search query"]', 'nativeeditorneedle');
    await clickText('Search', '.search-form button');
    await until(() => evaluate(`document.querySelector('.search-result')?.textContent.includes('Alpha')`), 'search result');
    await evaluate(`document.querySelector('.search-result').click()`); await pageIs('Alpha.md');
  });

  await check('reviewed rename updates incoming links and keeps page content', async () => {
    await clickText('Rename', '.note-path-row button');
    await setValue('dialog input:not([type="checkbox"])', 'Research/Alpha renamed.md');
    await clickText('Review changes', 'dialog button');
    await until(() => evaluate(`document.querySelector('.rename-changes')?.textContent.includes('Beta.md')`), 'rename link preview');
    await evaluate(`document.querySelector('dialog input[type="checkbox"]').click()`);
    await clickText('Rename and update links', 'dialog button'); await pageIs('Research/Alpha renamed.md');
    assert.equal(readFileSync(join(vault, 'Research', 'Alpha renamed.md'), 'utf8'), externalAlpha);
    assert(!existsSync(join(vault, 'Alpha.md')));
    assert.match(readFileSync(join(vault, 'Beta.md'), 'utf8'), /\[\[(?:Research\/)?Alpha renamed(?:\.md)?\]\]/);
    assert((await native('page_links', { path: 'Beta.md' })).outgoing.some((link) => link.path === 'Research/Alpha renamed.md'));
  });

  await check('create a Markdown page from the desktop', async () => {
    await evaluate(`document.querySelector('button[aria-label="Create a note"]').click()`);
    const betaSource = readFileSync(join(vault, 'Beta.md'), 'utf8');
    await setValue('dialog input', 'Beta.md');
    await setValue('dialog textarea', '# This must not overwrite Beta.\n');
    await clickText('Create note', 'dialog button');
    await until(() => evaluate(`document.querySelector('dialog [role="alert"]')?.textContent.toLowerCase().includes('already exists')`), 'existing note protection inside the form');
    assert.equal(readFileSync(join(vault, 'Beta.md'), 'utf8'), betaSource);
    await setValue('dialog input', 'New page.md');
    await setValue('dialog textarea', '# New page\n\nCreated through the native UI.\n');
    await clickText('Create note', 'dialog button');
    await until(() => evaluate(`!!document.querySelector('.cm-content')`), 'new note editor');
    assert.equal(readFileSync(join(vault, 'New page.md'), 'utf8'), '# New page\n\nCreated through the native UI.\n');
  });

  await check('install templates, create a typed row, and render table board calendar gallery', async () => {
    await clickText('Views', '.view-switcher button');
    await clickText('Install GTD templates');
    await until(() => evaluate(`document.querySelector('select[aria-label="Saved view"]')?.options.length >= 6`), 'installed saved views');
    const date = ready.pagePath.split('/').at(-1).replace(/\.md$/, '');
    const cell = (type, value) => ({ type, value });
    await native('create_database_row', { database: 'gtd', path: 'Tasks/Inbox thought.md', body: '# Inbox thought\n', cells: { status: cell('select', 'inbox'), kind: cell('select', 'action') } });
    await native('create_database_row', { database: 'gtd', path: 'Tasks/Native task.md', body: '# Native task\n', cells: { status: cell('select', 'next'), kind: cell('select', 'action'), contexts: cell('multi_select', ['@computer']), review_date: cell('date', date), priority: cell('number', 2) } });
    await native('create_database_row', { database: 'gtd', path: 'Tasks/Native project.md', body: '# Native project\n', cells: { status: cell('select', 'next'), kind: cell('select', 'project'), review_date: cell('date', date) } });
    for (const [id, selector, expected] of [['inbox', '.database-table', 'Inbox thought'], ['next_actions', '.database-board', 'Native task'], ['weekly_review', '.database-calendar', 'Native task'], ['projects', '.database-gallery', 'Native project']]) {
      await setValue('select[aria-label="Saved view"]', id);
      await until(() => evaluate(`document.querySelector(${JSON.stringify(selector)})?.textContent.includes(${JSON.stringify(expected)})`), `${id} layout`);
      await screenshot(`05-${id}`);
    }
    await setValue('select[aria-label="Saved view"]', 'inbox');
    await until(() => evaluate(`document.querySelector('.database-table')?.textContent.includes('Inbox thought')`), 'inbox row');
    await clickText('Edit', '.database-table button');
    await setValue('dialog select', 'waiting');
    await clickText('Save to Markdown', 'dialog button');
    await until(() => evaluate(`!document.querySelector('dialog[open]')`), 'property save');
    assert.match(readFileSync(join(vault, 'Tasks', 'Inbox thought.md'), 'utf8'), /status: waiting/);
    assert.match(readFileSync(join(vault, 'Tasks', 'Inbox thought.md'), 'utf8'), /# Inbox thought/);
  });

  await check('generic schema and view setup writes validated TOML', async () => {
    await clickText('Database & view setup');
    await clickText('Load or create definition', 'dialog button');
    await until(() => evaluate(`!!document.querySelector('dialog textarea')`), 'schema definition editor');
    await clickText('Save definition', 'dialog button');
    await until(() => evaluate(`!document.querySelector('dialog[open]')`), 'schema definition save');
    assert.match(readFileSync(join(vault, '.selenite', 'schemas', 'notes.toml'), 'utf8'), /kind = "folder"/);
    await clickText('Database & view setup');
    await setValue('dialog select', 'view');
    await setValue('dialog input', 'all_notes');
    await clickText('Load or create definition', 'dialog button');
    await until(() => evaluate(`!!document.querySelector('dialog textarea')`), 'view definition editor');
    await clickText('Save definition', 'dialog button');
    await until(() => evaluate(`!document.querySelector('dialog[open]')`), 'view definition save');
    assert.match(readFileSync(join(vault, '.selenite', 'views', 'all_notes.toml'), 'utf8'), /database = "notes"/);
  });

  await check('disabled AI configuration saves locally without provider requests', async () => {
    await clickText('Services & settings');
    await until(() => evaluate(`!!document.querySelector('.config-editor')`), 'local service settings');
    const config = `enabled = false\nbase_url = "${disabledEndpoint}"\nchat_model = "disabled"\nembedding_model = "disabled"\napi_key_env = "SELENITE_SMOKE_NO_KEY"\nmax_context_chars = 12000\nfolders = []\n`;
    await setValue('.config-editor', config);
    await clickText('Save configurations');
    await until(() => evaluate(`document.querySelector('.service-save')?.textContent.includes('Saved configuration')`), 'saved disabled provider config');
    assert.equal(readFileSync(join(vault, '.selenite', 'ai.toml'), 'utf8'), config);
    assert(!(await evaluate(`document.querySelector('[role="alert"]')?.textContent`)));
    await screenshot('06-provider-settings');
  });

  await check('failed settings comparison preserves external configuration and local draft', async () => {
    const configPath = join(vault, '.selenite', 'ai.toml');
    const current = readFileSync(configPath, 'utf8');
    const draft = `${current}# local unsaved comment\n`;
    const external = `${current}# external saved comment\n`;
    await setValue('.config-editor', draft);
    writeFileSync(configPath, external);
    await clickText('Save configurations');
    await until(() => evaluate(`!!document.querySelector('.services-panel [role="alert"]')`), 'settings conflict');
    assert.equal(readFileSync(configPath, 'utf8'), external);
    assert.equal(await evaluate(`document.querySelector('.config-editor').value`), draft);
    assert(await evaluate(`document.querySelector('.save-state').textContent.includes('Unsaved')`));
    await evaluate(`document.querySelector('.today-button').click()`);
    await clickText('Discard changes', 'dialog button'); await pageIs(ready.pagePath);
    await clickText('Services & settings');
    await until(() => evaluate(`!!document.querySelector('.config-editor')`), 'reloaded configuration after discard');
    assert.equal(await evaluate(`document.querySelector('.config-editor').value`), external);
  });

  await check('disabled sync, catalog, and AI reject before making network requests', async () => {
    const syncSource = `enabled = false\nprovider = "google_drive"\nendpoint = "${disabledEndpoint}"\ntoken_env = "SELENITE_SMOKE_NO_SYNC_KEY"\nfolder_id = "fixture-folder"\ninclude_metadata = false\n`;
    const pluginSource = `enabled = false\ncatalog_url = "${disabledEndpoint}catalog.json"\n\n[trusted_catalog_keys]\n\n[grants]\n`;
    await native('save_service_config', { service: 'sync', expectedSource: null, source: syncSource });
    await native('save_service_config', { service: 'plugins', expectedSource: null, source: pluginSource });
    for (const command of ['sync_plan', 'plugin_catalog']) {
      const error = await evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}).then(() => null, (error) => String(error))`, true);
      assert.match(error, /disabled/i, `${command} should reject disabled configuration`);
    }
    const expectedConfig = readFileSync(join(vault, '.selenite', 'ai.toml'), 'utf8');
    const error = await evaluate(`window.__TAURI_INTERNALS__.invoke('ai_index', ${JSON.stringify({ expectedConfig })}).then(() => null, (error) => String(error))`, true);
    assert.match(error, /disabled/i);
    assert.deepEqual(await native('list_installed_plugins'), []);
    assert.equal(remoteAttempts, 0, 'Disabled services must not connect to the local endpoint trap');
  });

  await check('community entitlement status and invalid token rejection preserve access', async () => {
    const status = await native('pro_status');
    assert.equal(status.active, false);
    assert.deepEqual(status.capabilities, []);
    const error = await evaluate(`window.__TAURI_INTERNALS__.invoke('install_entitlement', {document:'{"key_id":"unknown","payload":"{}","signature":"invalid"}'}).then(() => null, (error) => String(error))`, true);
    assert.equal(typeof error, 'string');
    assert(!existsSync(join(vault, '.selenite', 'entitlement.json')));
    assert.equal((await native('read_page', { path: 'New page.md' })).title, 'New page');
  });

  await check('renaming today preserves its source without recreating the original note', async () => {
    await evaluate(`document.querySelector('.today-button').click()`); await pageIs(ready.pagePath);
    const original = readFileSync(join(vault, ...ready.pagePath.split('/')), 'utf8');
    const target = ready.pagePath.replace(/\.md$/, '-renamed.md');
    await clickText('Rename', '.page-tools button');
    await setValue('dialog input:not([type="checkbox"])', target);
    await clickText('Review changes', 'dialog button');
    await until(() => evaluate(`!!document.querySelector('dialog input[type="checkbox"]')`), 'today rename review');
    await evaluate(`document.querySelector('dialog input[type="checkbox"]').click()`);
    await clickText('Rename and update links', 'dialog button'); await pageIs(target);
    assert.equal(readFileSync(join(vault, ...target.split('/')), 'utf8'), original);
    assert(!existsSync(join(vault, ...ready.pagePath.split('/'))), 'Rename must not create another daily note as a response side effect');
    assert(await evaluate(`![...document.querySelectorAll('.file-row')].some(node => node.title === ${JSON.stringify(ready.pagePath)})`));
  });

  await check('no unhandled WebView exceptions', async () => { assert.deepEqual(runtimeErrors, []); });
  await client.call('Runtime.evaluate', { expression: `void window.__TAURI_INTERNALS__.invoke('smoke_exit')` }).catch(() => {});
  await Promise.race([closed, delay(5000).then(() => { throw new Error('Native smoke exit timed out'); })]);
} catch (error) {
  failure = error;
  if (client) await screenshot('failure').catch(() => {});
} finally {
  client?.socket.close();
  if (child.exitCode === null) child.kill();
  await new Promise((done) => networkTrap.close(done));
  writeFileSync(join(output, 'process.log'), logs.join(''), { flag: 'wx' });
  writeFileSync(join(output, 'summary.json'), JSON.stringify({ executable, vault, ready, checks, runtimeErrors, remoteAttempts, failure: failure ? String(failure.stack || failure) : null }, null, 2), { flag: 'wx' });
  console.log(`Evidence: ${output}`);
}
if (failure) throw failure;
