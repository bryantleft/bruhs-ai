// BRUHS_API_URL=http://127.0.0.1:8897 trunk build --dist ../target/unified-threads-dist
// node bruhs-web/tests/unified_threads_browser.mjs
// Uses fresh browser contexts, synthetic local records, and mocked agent requests.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_THREADS_DIST ?? path.join(root, 'target/unified-threads-dist'));
const output = path.join(root, 'target/unified-threads-qa');
const port = Number(process.env.BRUHS_THREADS_PORT ?? 8897);
const origin = `http://127.0.0.1:${port}`;
assert.ok((await readdir(dist)).some(file => file.endsWith('_bg.wasm')), 'Build the actual WASM app first');
await mkdir(output, { recursive: true });

async function playwrightModule() {
  if (process.env.BRUHS_PLAYWRIGHT_MODULE) return import(pathToFileURL(process.env.BRUHS_PLAYWRIGHT_MODULE).href);
  try { return await import('playwright'); } catch { /* Reuse an installed test dependency. */ }
  for (const entry of await readdir(path.join(homedir(), '.npm/_npx')).catch(() => [])) {
    try { return await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); }
    catch { /* Other cached tools need not contain Playwright. */ }
  }
  throw new Error('Install Playwright or set BRUHS_PLAYWRIGHT_MODULE');
}
const { chromium } = await playwrightModule();
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, origin).pathname;
  if (pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' });
    response.write(': isolated fixture\n\n');
    return;
  }
  if (pathname.startsWith('/api/')) { response.writeHead(500); response.end('Unmocked API request'); return; }
  const file = path.resolve(dist, `.${pathname === '/' ? '/index.html' : decodeURIComponent(pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.wasm': 'application/wasm', '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(port, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) });
const report = [];

async function record(page, store, id) {
  return page.evaluate(({ store, id }) => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result;
      const read = db.transaction(store).objectStore(store).get(id);
      read.onsuccess = () => { db.close(); resolve(read.result); };
      read.onerror = () => { db.close(); reject(read.error); };
    };
  }), { store, id });
}
async function sidebar(page, open) {
  const toggle = page.getByRole('button', { name: 'Toggle sidebar (⌘B)', exact: true });
  if ((await toggle.getAttribute('aria-expanded') === 'true') === open) return;
  const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
  if (!open && await scrim.isVisible()) {
    await scrim.click({ position: { x: page.viewportSize().width - 12, y: 60 } });
  } else await toggle.click();
}
async function waitSaved(page, id, project) {
  await page.waitForFunction(({ id, project }) => new Promise(resolve => {
    const open = indexedDB.open('bruhs');
    open.onsuccess = () => {
      const db = open.result;
      const read = db.transaction('threads').objectStore('threads').get(id);
      read.onsuccess = () => { db.close(); resolve(read.result?.project_name === project); };
    };
  }), { id, project });
}
async function chooseFolder(page) {
  await page.getByRole('button', { name: 'Choose project folder', exact: true }).first().click();
  const dialog = page.getByRole('dialog', { name: 'Choose project folder', exact: true });
  await dialog.getByRole('button', { name: 'Select this folder', exact: true }).click();
  return dialog;
}

try {
  for (const cell of [
    { name: 'desktop', width: 1280, height: 900, legacyMode: 'code' },
    { name: 'compact', width: 640, height: 480, legacyMode: 'chat' },
    { name: 'touch', width: 390, height: 844, hasTouch: true, legacyMode: 'code' },
  ]) {
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: !!cell.hasTouch });
    const state = { cell: cell.name, checks: [], turns: [], errors: [], unexpected: [] };
    report.push(state);
    const check = (ok, label) => { assert.ok(ok, `${cell.name}: ${label}`); state.checks.push(label); console.log(`${cell.name}: ${label}`); };
    await context.addInitScript(({ origin, legacyMode }) => {
      if (location.origin !== origin || localStorage.getItem('unified-threads-seeded')) return;
      localStorage.setItem('unified-threads-seeded', '1');
      localStorage.setItem('bruhs.workspaceMode', legacyMode);
      localStorage.setItem('bruhs.model', 'claude/claude-sonnet-4-6');
      localStorage.setItem('bruhs.taskModels', JSON.stringify({ prompt_suggestions: false }));
      localStorage.setItem('bruhs.permissionMode', 'plan');
      localStorage.setItem('bruhs.activeProject', JSON.stringify({ name: 'Stale project', path: '/fixture/stale' }));
      const open = indexedDB.open('bruhs', 1);
      open.onupgradeneeded = () => {
        open.result.createObjectStore('threads', { keyPath: 'id' });
        open.result.createObjectStore('messages', { keyPath: 'thread_id' });
      };
      open.onsuccess = () => {
        const db = open.result;
        const tx = db.transaction(['threads', 'messages'], 'readwrite');
        for (const [id, name, folder] of [['general', '', null], ['alpha', 'Alpha', '/fixture/alpha'], ['beta', 'Beta', '/fixture/beta']]) {
          tx.objectStore('threads').put({ id, title: `Thread ${id}`, project_name: name, project_path: folder, updated_at: Date.now(), pinned: false, settled: false, session_id: id === 'general' ? 'old-general-session' : null, model: 'claude/claude-sonnet-4-6' });
        }
        tx.objectStore('messages').put({ thread_id: 'general', messages: [
          { id: 'question', role: 'user', content: 'Keep the architecture discussion' },
          { id: 'answer', role: 'assistant', content: 'Keep the design decision' },
        ] });
        tx.oncomplete = () => db.close();
        tx.onabort = () => db.close();
      };
    }, { origin, legacyMode: cell.legacyMode });
    await context.route('**/*', async route => {
      const request = route.request();
      const url = new URL(request.url());
      if (!url.pathname.startsWith('/api/')) {
        if (url.origin === origin) return route.continue();
        state.unexpected.push(request.url());
        return route.abort();
      }
      const headers = { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, POST, OPTIONS', 'Cache-Control': 'no-store' };
      const json = value => route.fulfill({ status: 200, contentType: 'application/json', headers, body: JSON.stringify(value) });
      if (request.method() === 'OPTIONS') return json({});
      if (url.pathname === '/api/agent' && request.method() === 'POST') {
        state.turns.push(request.postDataJSON());
        if (state.turns.at(-1).prompt.includes('Hold this turn')) await new Promise(resolve => { state.release = resolve; });
        const events = [
          { type: 'init', session_id: `new-session-${state.turns.length}`, model: 'claude-sonnet-4-6', tools: [], mcp_servers: [] },
          { type: 'text', text: `Fixture response ${state.turns.length}` },
          { type: 'result', result: `Fixture response ${state.turns.length}`, session_id: `new-session-${state.turns.length}`, duration_ms: 1, usage: { input_tokens: 1, output_tokens: 1, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 }, is_error: false },
        ];
        return route.fulfill({ status: 200, contentType: 'text/event-stream', headers, body: events.map(event => `data: ${JSON.stringify({ envelope: 'query', ...event })}\n\n`).join('') });
      }
      if (url.pathname === '/api/people/profiles' && request.method() === 'POST') return json([]);
      if (url.pathname === '/api/agent/git-branches') return json({ branches: ['main'], current: 'main' });
      if (url.pathname === '/api/agent/branch-pull') return json({ signed_in: false, pull: null });
      if (url.pathname === '/api/agent/git-status') return json({ branch: 'main', ahead: 0, behind: 0, dirty: [] });
      if (url.pathname === '/api/agent/list-dir') return json({ current: '/fixture/attached', name: 'Attached', parent: '/fixture', dirs: [] });
      if (request.method() !== 'GET') { state.unexpected.push(`${request.method()} ${url.pathname}`); return json({}); }
      if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
      if (url.pathname === '/api/agent/validate-path') return json({ path: '/fixture/attached', name: 'Attached', valid: true });
      if (/\/auth\//.test(url.pathname)) return json({ status: 'authenticated', authenticated: true });
      if (url.pathname === '/api/agent/providers') return json({ providers: [{ provider: 'claude', label: 'Claude Code', available: true, authenticated: true, supports_custom_models: true, capabilities: { access_modes: ['default', 'auto-accept', 'yolo', 'plan'], effort_values: ['low', 'medium', 'high'], custom_models: true, app_tools: true, second_opinion: true, prompt_suggestions: false } }] });
      if (url.pathname === '/api/agent/models') return json({ models: [{ id: 'claude/claude-sonnet-4-6', name: 'Claude Sonnet', provider: 'claude', group: 'Claude', context_window: 200000, supports_images: true }], source: 'fixture', stale: false });
      if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
      if (url.pathname === '/api/people/versions') return json({ entries: [], next_after: null });
      if (/(meetings|people|credentials|tasks|threads|skills)$/.test(url.pathname)) return json([]);
      return json({});
    });
    const page = await context.newPage();
    page.setDefaultTimeout(15000);
    page.on('pageerror', error => state.errors.push(error.message));
    try {
      await page.goto(`${origin}/#thread/beta`);
      await page.getByRole('heading', { name: 'Thread beta', exact: true }).waitFor();
      check(page.url().endsWith('#thread/beta'), 'Startup preserves a deep link to a non-first thread');
      await page.locator('#chat-input').waitFor();
      await sidebar(page, true);
      check(await page.locator('[data-thread-id]').count() === 3, 'Legacy mode preferences leave every thread visible');
      check(await page.getByRole('button', { name: 'Code', exact: true }).count() === 0, 'No Code workspace switch');
      await page.locator('[data-thread-id="general"]').click();
      await page.waitForURL('**/#thread/general');
      await sidebar(page, true);
      await page.locator('[data-thread-id="alpha"]').click();
      await page.waitForURL('**/#thread/alpha');
      await page.goBack();
      await page.getByRole('heading', { name: 'Thread general', exact: true }).waitFor();
      check(page.url().endsWith('#thread/general'), 'Browser Back selects the previous conversation');
      await page.goForward();
      await page.getByRole('heading', { name: 'Thread alpha', exact: true }).waitFor();
      check(page.url().endsWith('#thread/alpha'), 'Browser Forward selects the next conversation');
      await sidebar(page, true);
      await page.locator('[data-thread-id="general"]').click();
      await page.waitForURL('**/#thread/general');
      await sidebar(page, true);
      await page.getByRole('navigation', { name: 'Threads', exact: true }).getByRole('button', { name: 'All threads', exact: true }).click();
      await page.getByRole('menuitem', { name: 'Alpha', exact: true }).click();
      await page.waitForFunction(() => document.querySelectorAll('[data-thread-id]').length === 1);
      check(await page.locator('[data-thread-id="alpha"]').count() === 1, 'Project filter narrows the unified list');
      await page.getByRole('navigation', { name: 'Threads', exact: true }).getByRole('button', { name: 'Alpha', exact: true }).click();
      await page.getByRole('menuitem', { name: 'No project', exact: true }).click();
      check(await page.locator('[data-thread-id="general"]').count() === 1, 'No project filter finds general conversations');
      await page.getByRole('navigation', { name: 'Threads', exact: true }).getByRole('button', { name: 'No project', exact: true }).click();
      await page.getByRole('menuitem', { name: 'All threads', exact: true }).click();
      const order = await page.locator('[data-thread-id]').evaluateAll(rows => rows.map(row => row.dataset.threadId));
      const destination = order[0] === 'general' ? order.at(-1) : order[0];
      await page.locator('[data-thread-id="general"]').focus();
      await page.keyboard.press(order[0] === 'general' ? 'End' : 'Home');
      await page.waitForFunction(id => document.activeElement?.getAttribute('data-thread-id') === id, destination);
      await page.keyboard.press('Enter');
      await sidebar(page, true);
      await page.waitForFunction(id => document.querySelector(`[data-thread-id="${id}"]`)?.getAttribute('aria-current') === 'page', destination);
      check(true, 'Keyboard navigation reaches project threads from a general thread');
      await sidebar(page, true);
      await page.locator('[data-thread-id="general"]').click();
      await sidebar(page, true);
      await page.getByRole('button', { name: 'New thread (⌘⇧O)', exact: true }).click();
      await sidebar(page, false);
      check(await page.getByRole('dialog', { name: 'Choose project folder', exact: true }).count() === 0, 'New thread needs no project picker');
      await page.locator('#chat-input').fill('A projectless follow-up');
      await page.getByRole('button', { name: 'Send message', exact: true }).click();
      await page.getByText('Fixture response 1', { exact: true }).first().waitFor();
      check(!state.turns[0].project_path, 'General threads ignore the stale global project');

      await sidebar(page, true);
      await page.locator('[data-thread-id="alpha"]').click();
      await sidebar(page, true);
      await page.getByRole('button', { name: 'New thread (⌘⇧O)', exact: true }).click();
      await sidebar(page, false);
      check(await page.getByRole('button', { name: 'Choose project folder', exact: true }).first().textContent().then(text => text.includes('Alpha')), 'New thread inherits its current project');

      await sidebar(page, true);
      await page.locator('[data-thread-id="general"]').click();
      await sidebar(page, false);
      await page.getByText('Keep the design decision', { exact: true }).waitFor();
      await page.locator('#chat-input').fill('Build the discussed design');
      const before = await record(page, 'threads', 'general');
      await (await chooseFolder(page)).waitFor({ state: 'hidden' });
      await waitSaved(page, 'general', 'Attached');
      const attached = await record(page, 'threads', 'general');
      check(attached.id === before.id && attached.title === before.title, 'Attaching a folder preserves thread identity and title');
      check(!attached.session_id && attached.fresh_session && attached.import_context.includes('Keep the design decision'), 'The new session carries the earlier conversation');
      check(await page.locator('#chat-input').inputValue() === 'Build the discussed design', 'Attaching preserves the composer draft');
      await page.getByRole('button', { name: 'Send message', exact: true }).click();
      await page.getByText('Fixture response 2', { exact: true }).first().waitFor();
      check(state.turns[1].project_path === '/fixture/attached' && !state.turns[1].session_id, 'The next turn uses the chosen folder and a new session');
      check(state.turns[1].prompt.includes('Keep the architecture discussion') && state.turns[1].prompt.includes('Keep the design decision'), 'The agent receives the existing conversation');
      check(state.turns[1].permission_mode === state.turns[0].permission_mode, 'Attaching a project preserves the access mode');
      await page.locator('#chat-input').fill('Hold this turn');
      await page.getByRole('button', { name: 'Send message', exact: true }).click();
      await page.getByRole('button', { name: 'Interrupt', exact: true }).waitFor();
      await page.getByRole('button', { name: 'Choose project folder', exact: true }).first().click();
      const busyDialog = page.getByRole('dialog', { name: 'Choose project folder', exact: true });
      await busyDialog.getByRole('button', { name: 'No project', exact: true }).click();
      await busyDialog.getByText('Wait for this thread to finish before changing its project.', { exact: true }).waitFor();
      check((await record(page, 'threads', 'general')).project_name === 'Attached', 'An active turn cannot be moved to another folder');
      await page.keyboard.press('Escape');
      state.release();
      await page.getByText('Fixture response 3', { exact: true }).first().waitFor();
      await page.reload();
      await page.locator('#chat-input').waitFor();
      await page.getByRole('heading', { name: 'Thread general', exact: true }).waitFor();
      await sidebar(page, false);
      check(page.url().endsWith('#thread/general'), 'Reload restores the selected conversation directly');
      await page.getByText('Keep the design decision', { exact: true }).waitFor();
      check((await record(page, 'threads', 'general')).project_name === 'Attached', 'Project attachment survives reload');
      await page.getByRole('button', { name: 'Choose project folder', exact: true }).first().click();
      await page.getByRole('dialog', { name: 'Choose project folder', exact: true }).getByRole('button', { name: 'No project', exact: true }).click();
      await waitSaved(page, 'general', '');
      const detached = await record(page, 'threads', 'general');
      check(!detached.project_path && detached.import_context.includes('Keep the design decision'), 'Removing a folder keeps the conversation');
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'No horizontal viewport overflow');
      check(await page.getByRole('heading', { name: 'Thread general', exact: true }).isVisible(), 'The thread title remains visible beside project controls');
      check(state.errors.length === 0, 'No browser runtime errors');
      check(state.unexpected.length === 0, 'Only isolated fixture requests');
      await page.screenshot({ path: path.join(output, `${cell.name}.png`) });
    } catch (error) {
      state.failure = error.stack;
      await page.screenshot({ path: path.join(output, `${cell.name}-failure.png`) });
      throw error;
    } finally {
      await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
      await context.close();
    }
  }
  console.log(JSON.stringify({ cells: report.length, checks: report.reduce((sum, row) => sum + row.checks.length, 0), output }));
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
