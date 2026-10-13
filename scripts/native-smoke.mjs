// Windows native WebView smoke test. Retains all evidence inside this repository.
// Usage: node scripts/native-smoke.mjs .local/desktop-benchmark/vault [runs]
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';
import assert from 'node:assert/strict';

const root = realpathSync(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
function contained(path) {
  const absolute = realpathSync(resolve(root, path));
  const part = relative(root, absolute);
  assert(!part.startsWith('..') && !isAbsolute(part), 'Smoke paths must remain in the repository');
  return absolute;
}
const vault = contained(process.argv[2] ?? '.local/desktop-benchmark/vault');
const executable = contained('target/release/selenite-desktop.exe');
const count = Number(process.argv[3] ?? 5);
assert(Number.isSafeInteger(count) && count > 0 && count <= 20);
const output = join(root, '.local', 'native-smoke', `${Date.now()}-${process.pid}`);
mkdirSync(output, { recursive: true });
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
async function until(operation, label, timeout = 30000) {
  const started = performance.now();
  while (performance.now() - started < timeout) {
    const result = await operation();
    if (result) return result;
    await delay(10);
  }
  throw new Error(`Timed out waiting for ${label}`);
}
async function freePort() {
  const server = createServer();
  await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
  const port = server.address().port;
  await new Promise((done) => server.close(done));
  return port;
}
async function connect(url) {
  const socket = new WebSocket(url);
  await new Promise((done, reject) => { socket.addEventListener('open', done, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  let sequence = 0;
  const pending = new Map();
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  return {
    socket,
    call(method, params = {}) {
      const id = ++sequence;
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 10000);
        pending.set(id, { resolve, reject, timer });
        socket.send(JSON.stringify({ id, method, params }));
      });
    },
  };
}
const results = [];
for (let run = 0; run < count; run++) {
  const report = join(output, `ready-${run + 1}.json`);
  const port = await freePort();
  const launched = performance.now();
  const child = spawn(executable, [], {
    cwd: root, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env, TEMP: join(root, '.local', 'tmp'), TMP: join(root, '.local', 'tmp'),
      SELENITE_VAULT: vault, SELENITE_SMOKE_REPORT: report,
      SELENITE_WEBVIEW_DATA_DIR: join(output, 'webview-profile'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1`,
    },
  });
  const messages = [];
  child.stdout.on('data', (data) => messages.push(data.toString()));
  child.stderr.on('data', (data) => messages.push(data.toString()));
  let failure;
  child.once('error', (error) => { failure = error; });
  const closed = new Promise((done) => child.once('close', done));
  let client;
  try {
    const ready = await until(() => {
      if (failure) throw failure;
      if (child.exitCode !== null) throw new Error(`Native process exited ${child.exitCode}: ${messages.join('')}`);
      if (!existsSync(report)) return null;
      try { return JSON.parse(readFileSync(report, 'utf8')); } catch { return null; }
    }, 'the native renderer readiness report');
    const processToReadyMs = performance.now() - launched;
    assert.equal(ready.processId, child.pid);
    assert.equal(ready.surface, 'native-tauri-webview');
    const target = await until(async () => {
      try {
        const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((response) => response.json());
        return targets.find((target) => target.type === 'page' && target.webSocketDebuggerUrl);
      } catch { return null; }
    }, 'the application WebView');
    client = await connect(target.webSocketDebuggerUrl);
    async function evaluate(expression, awaitPromise = false) {
      const result = await client.call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise });
      assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
      return result.result.value;
    }
    const rendered = await evaluate(`({ path: document.querySelector('[data-testid="note-preview"]')?.dataset.pagePath, title: document.title, tree: !!document.querySelector('[data-testid="file-tree"]'), error: document.querySelector('[data-testid="app-error"]')?.textContent, text: document.querySelector('[data-testid="note-preview"]')?.textContent })`);
    assert.equal(rendered.path, ready.pagePath);
    assert(rendered.tree);
    assert(!rendered.error, rendered.error);
    assert(rendered.text.trim().length > 0);
    if (run === 0) {
      const screenshot = await client.call('Page.captureScreenshot', { format: 'png' });
      writeFileSync(join(output, 'native-today.png'), Buffer.from(screenshot.data, 'base64'));
      const clicked = await evaluate(`(() => { const folder = [...document.querySelectorAll('.folder-row')].find(node => node.title !== 'Journal'); if (folder?.getAttribute('aria-expanded') === 'false') folder.click(); return !!folder; })()`);
      assert(clicked, 'Expected another fixture folder');
      await until(async () => evaluate(`!![...document.querySelectorAll('.file-row')].find(node => node.title !== ${JSON.stringify(ready.pagePath)})`), 'expanded fixture folder');
      const cancelled = await evaluate(`(async () => { const files = [...document.querySelectorAll('.file-row')]; const current = files.find(node => node.title === ${JSON.stringify(ready.pagePath)}); const other = files.find(node => node.title !== ${JSON.stringify(ready.pagePath)}); other.click(); current.click(); await new Promise(done => setTimeout(done, 400)); return document.querySelector('[data-testid="note-preview"]')?.dataset.pagePath; })()`, true);
      assert.equal(cancelled, ready.pagePath, 'Returning to the displayed page must cancel an in-flight navigation');
      await until(async () => evaluate(`(() => { const file = [...document.querySelectorAll('.file-row')].find(node => node.title !== ${JSON.stringify(ready.pagePath)}); if (!file || file.disabled) return false; file.click(); return true; })()`), 'another note button');
      await until(async () => evaluate(`document.querySelector('[data-testid="note-preview"]')?.dataset.pagePath !== ${JSON.stringify(ready.pagePath)}`), 'another Markdown preview');
      await evaluate(`document.querySelector('.today-button').click()`);
      await until(async () => evaluate(`document.querySelector('[data-testid="note-preview"]')?.dataset.pagePath === ${JSON.stringify(ready.pagePath)}`), 'return to today');
      const rejected = await evaluate(`window.__TAURI_INTERNALS__.invoke('read_page', {path: '../outside.md'}).then(() => false, () => true)`, true);
      assert.equal(rejected, true, 'Native command must reject path traversal');
    }
    const result = { run: run + 1, profile: run === 0 ? 'fresh' : 'reused', ...ready, processToReadyMs, rendered };
    results.push(result);
    console.log(JSON.stringify(result));
    // Deliberately do not await the command response: exit can close IPC first.
    await client.call('Runtime.evaluate', { expression: `void window.__TAURI_INTERNALS__.invoke('smoke_exit')` }).catch(() => {});
    await Promise.race([closed, delay(5000).then(() => { throw new Error('Native smoke exit timed out'); })]);
  } finally {
    client?.socket.close();
    if (child.exitCode === null) child.kill(); // Only the exact child created above.
    writeFileSync(join(output, `process-${run + 1}.log`), messages.join(''));
  }
}
writeFileSync(join(output, 'summary.json'), JSON.stringify({ vault, results }, null, 2));
console.log(`Evidence: ${output}`);
