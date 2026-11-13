// Build with BRUHS_API_URL=http://127.0.0.1:8791, then run with
// BRUHS_PROVIDER_UI_DIST=/path/to/dist node bruhs-web/tests/provider_models_browser.mjs.
// Fresh contexts and mocked APIs only: no live providers, saved profiles, or user data.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_PROVIDER_UI_DIST ?? path.join(root, 'target/provider-ui-dist'));
const output = path.resolve(process.env.BRUHS_PROVIDER_UI_OUTPUT ?? path.join(root, 'target/provider-ui-qa'));
const port = Number(process.env.BRUHS_PROVIDER_UI_PORT ?? 8791);
const origin = `http://127.0.0.1:${port}`;
const index = await readFile(path.join(dist, 'index.html'), 'utf8');
assert.ok(index.includes('.wasm') && (await readdir(dist)).some(file => file.endsWith('_bg.wasm')), 'Serve the production Trunk bundle');
for (const file of ['bruhs-web/src/components/model_picker.rs', 'bruhs-web/src/components/provider.rs', 'bruhs-web/src/components/settings_page.rs', 'bruhs-web/src/components/role_settings.rs', 'bruhs-web/src/components/auth_banner.rs']) {
  assert.ok((await stat(path.join(dist, 'index.html'))).mtimeMs >= (await stat(path.join(root, file))).mtimeMs, `Rebuild stale provider UI assets after changing ${file}`);
}
await mkdir(output, { recursive: true });

async function playwrightModule() {
  if (process.env.BRUHS_PLAYWRIGHT_MODULE) return import(pathToFileURL(process.env.BRUHS_PLAYWRIGHT_MODULE).href);
  try { return await import('playwright'); } catch { /* Reuse an installed browser harness. */ }
  const cache = path.join(homedir(), '.npm/_npx');
  for (const entry of await readdir(cache).catch(() => [])) {
    try { return await import(pathToFileURL(path.join(cache, entry, 'node_modules/playwright/index.mjs')).href); }
    catch { /* Other cached packages need not contain Playwright. */ }
  }
  throw new Error('Install Playwright or set BRUHS_PLAYWRIGHT_MODULE.');
}

const { chromium } = await playwrightModule();
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, origin).pathname;
  if (pathname === '/__provider-fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' });
    response.write(': isolated provider fixture\n\n');
    const timer = setInterval(() => response.write(': connected\n\n'), 1000);
    request.on('close', () => clearInterval(timer));
    return;
  }
  // Unmocked API requests fail locally instead of contacting an actual app server.
  if (pathname.startsWith('/api/')) { response.writeHead(500); response.end('API request escaped the isolated fixture'); return; }
  const file = path.resolve(dist, `.${pathname === '/' ? '/index.html' : decodeURIComponent(pathname)}`);
  if (!file.startsWith(`${dist}${path.sep}`)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.wasm': 'application/wasm', '.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end('Missing isolated provider fixture asset'); }
});
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
const browser = await chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) }).catch(async error => {
  await new Promise(resolve => server.close(resolve));
  throw error;
});
const passed = [];
const baselineConsole = new Set();
const checks = [];
const check = (condition, label) => { assert.ok(condition, label); passed.push(label); if (process.env.BRUHS_BROWSER_DEBUG) console.log(label); };

async function fixture(context) {
  const state = { turns: [], requests: [], unexpected: [], errors: [] };
  await context.addInitScript(() => {
    if (window.top !== window || localStorage.getItem('provider-fixture-initialized')) return;
    localStorage.setItem('provider-fixture-initialized', '1');
    localStorage.setItem('bruhs.taskModels', JSON.stringify({ prompt_suggestions: true }));
  });
  await context.route('**/*', async route => {
    const request = route.request();
    const url = new URL(request.url());
    state.requests.push(`${request.method()} ${url.pathname}`);
    if (!url.pathname.startsWith('/api/')) {
      if (url.origin === origin) return route.continue();
      state.unexpected.push(request.url());
      return route.abort();
    }
    const json = value => route.fulfill({ status: 200, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, POST, OPTIONS', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
    if (request.method() === 'OPTIONS') return json({});
    if (url.pathname === '/api/agent' && request.method() === 'POST') {
      const body = request.postDataJSON();
      state.turns.push(body);
      const number = state.turns.length;
      const runtime = body.model.split('/')[0];
      const session = body.session_id ?? `fixture-${runtime}-${number}`;
      const usage = { input_tokens: 1, output_tokens: 1, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 };
      const events = [
        { type: 'init', session_id: session, model: body.model, tools: [], mcp_servers: [] },
        { type: 'text', text: `Provider fixture response ${number}.` },
        { type: 'result', result: `Provider fixture response ${number}.`, session_id: session, duration_ms: 1, usage, is_error: false },
      ];
      return route.fulfill({ status: 200, contentType: 'text/event-stream', headers: { 'Cache-Control': 'no-store', 'Access-Control-Allow-Origin': origin }, body: events.map(event => `data: ${JSON.stringify({ envelope: 'query', ...event })}\n\n`).join('') });
    }
    if (url.pathname === '/api/people/profiles' && request.method() === 'POST') return json([]);
    if (request.method() !== 'GET') {
      state.unexpected.push(`${request.method()} ${url.pathname}`);
      return route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'Live actions are forbidden in this fixture.' }) });
    }
    if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__provider-fixture-stream` });
    if (url.pathname === '/api/people/versions') return json({ entries: [], next_after: null });
    if (/\/auth\//.test(url.pathname)) return json({ authenticated: false, user: null });
    if (/\/(meetings|people|credentials|tasks|threads|skills|models)$/.test(url.pathname)) return json([]);
    if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
    return json({});
  });
  return state;
}

async function dismissSidebar(page, cell) {
  if (cell.width >= 768) return;
  const sidebar = page.getByRole('complementary', { name: 'Sidebar navigation', exact: true });
  if (await sidebar.isVisible()) {
    const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
    if (!await scrim.isVisible()) return;
    if (cell.hasTouch) await scrim.tap({ position: { x: cell.width - 12, y: 60 } });
    else await scrim.click({ position: { x: cell.width - 12, y: 60 } });
    await sidebar.waitFor({ state: 'hidden' });
  }
}
async function chatReady(page, cell) {
  await page.locator('#chat-input').waitFor({ state: 'visible' });
  await dismissSidebar(page, cell);
}
async function picker(page) {
  await page.locator('[data-model-trigger]').click();
  const dialog = page.getByRole('dialog', { name: 'Choose model', exact: true });
  await dialog.waitFor({ state: 'visible' });
  return dialog;
}
async function choose(page, id, method) {
  const dialog = await picker(page);
  const search = dialog.getByRole('textbox', { name: 'Search models', exact: true });
  await search.fill(id);
  const option = dialog.getByRole('option', { name: `Use ${id}`, exact: false });
  await option.waitFor({ state: 'visible' });
  if (method === 'keyboard') await search.press('Enter');
  else if (await page.evaluate(() => navigator.maxTouchPoints > 0)) await option.tap();
  else await option.click();
  await dialog.waitFor({ state: 'hidden' });
  await page.waitForFunction(id => localStorage.getItem('bruhs.model') === id, id);
  check((await page.locator('[data-model-trigger]').getAttribute('title')).includes(id), `${method}: composer selects ${id}`);
}
async function send(page, state, prompt, number) {
  await page.locator('#chat-input').fill(prompt);
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await page.getByText(`Provider fixture response ${number}.`, { exact: true }).first().waitFor();
  await page.getByRole('button', { name: 'Stop generation', exact: true }).waitFor({ state: 'hidden' });
  assert.equal(state.turns.length, number, 'A send produces exactly one mocked provider request');
}
async function inViewport(page, locator, label) {
  await locator.scrollIntoViewIfNeeded();
  const box = await locator.boundingBox();
  const viewport = page.viewportSize();
  check(box && box.width > 0 && box.height > 0 && box.x >= -1 && box.y >= -1 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1, label);
}

try {
  for (const cell of [{ name: 'desktop', width: 1280, height: 900 }, { name: 'compact', width: 640, height: 480 }, { name: 'touch', width: 390, height: 844, hasTouch: true, isMobile: true }]) {
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: !!cell.hasTouch, isMobile: !!cell.isMobile });
    const state = await fixture(context);
    const page = await context.newPage();
    page.on('pageerror', error => state.errors.push(error.message));
    page.on('console', message => {
      if (message.type() !== 'error') return;
      const text = message.text();
      if (/^The source list for the Content Security Policy directive '(img-src|media-src)' contains an invalid source: 'https?:\/\/\[::1\]:\*'\. It will be ignored\.$/.test(text)) baselineConsole.add(text);
      else state.errors.push(text);
      if (process.env.BRUHS_BROWSER_DEBUG) console.error(cell.name, text);
    });
    try {
      await page.goto(`${origin}/#/chat`);
      await chatReady(page, cell);
      let dialog = await picker(page);
      for (const label of ['Anthropic', 'OpenAI Codex', 'Cursor', 'Antigravity', 'Grok', 'OpenCode']) {
        check(await dialog.getByText(label, { exact: true }).count() === 1, `${cell.name}: ${label} runtime appears once`);
      }
      for (const label of ['Cursor default', 'Antigravity default', 'Grok default', 'OpenCode default']) {
        check(await dialog.getByRole('option', { name: new RegExp(label) }).count() === 1, `${cell.name}: ${label} is selectable`);
      }
      await page.waitForFunction(() => [...document.querySelectorAll('[role="dialog"][aria-label="Choose model"] img')].every(image => image.complete && image.naturalWidth > 0));
      check(true, `${cell.name}: all provider glyphs load from the local bundle`);
      await dialog.getByRole('textbox', { name: 'Search models', exact: true }).fill('unknown/model');
      await dialog.getByText(/^No models match\./).waitFor({ state: 'visible' });
      check(await dialog.getByRole('option').count() === 0, `${cell.name}: unknown runtime cannot be selected`);
      await dialog.getByRole('textbox', { name: 'Search models', exact: true }).press('Escape');
      const firstModel = 'cursor/fixture-model';
      await choose(page, firstModel, 'keyboard');
      await page.getByRole('button', { name: 'Log in with Claude', exact: true }).waitFor({ state: 'hidden' });
      check(!await page.getByText('Claude is not authenticated.', { exact: true }).isVisible(), `${cell.name}: Cursor does not inherit an unrelated Claude authentication warning`);
      await page.reload();
      await chatReady(page, cell);
      check((await page.locator('[data-model-trigger]').getAttribute('title')).includes(firstModel), `${cell.name}: custom runtime/model survives reload`);
      await send(page, state, 'Provider fixture turn one.', 1);
      await send(page, state, 'Provider fixture turn two.', 2);
      check(state.turns[0].model === firstModel && state.turns[1].model === firstModel, `${cell.name}: both requests preserve the explicit runtime namespace`);
      check(state.turns[1].session_id === 'fixture-cursor-1' && state.turns[1].thread_id === state.turns[0].thread_id, `${cell.name}: the second turn resumes its own runtime session`);
      const customModel = 'opencode/local/fixture-model';
      await choose(page, customModel, 'pointer');
      check(!await page.getByText('Claude is not authenticated.', { exact: true }).isVisible(), `${cell.name}: OpenCode does not inherit an unrelated Claude authentication warning`);
      await send(page, state, 'Provider fixture turn three.', 3);
      check(state.turns[2].model === customModel && !state.turns[2].session_id && state.turns[2].fresh_session === true, `${cell.name}: changing runtimes starts a fresh session without reusing the Cursor session`);
      check(state.turns[2].thread_id === state.turns[0].thread_id, `${cell.name}: runtime switching retains the thread`);

      await page.goto(`${origin}/#/settings`);
      await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor();
      await dismissSidebar(page, cell);
      const defaultModel = page.getByRole('button', { name: 'Default model', exact: true });
      await defaultModel.click();
      let listbox = page.getByRole('listbox');
      let customOption = listbox.getByRole('option', { name: /fixture-model/ });
      check(await customOption.count() === 1 && await customOption.getAttribute('aria-selected') === 'true', `${cell.name}: default settings retain the current custom model`);
      await customOption.click();
      await page.getByRole('button', { name: 'New role', exact: true }).click();
      await page.getByRole('textbox', { name: 'Role name', exact: true }).fill('Provider fixture role');
      await page.getByRole('textbox', { name: 'Role name', exact: true }).press('Tab');
      await page.getByRole('button', { name: 'Role model', exact: true }).click();
      await page.getByRole('listbox').getByRole('option', { name: /fixture-model/ }).click();
      await page.waitForFunction(id => JSON.parse(localStorage.getItem('bruhs.roles')).roles.some(role => role.name === 'Provider fixture role' && role.model === id), customModel);
      await page.reload();
      await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor();
      await dismissSidebar(page, cell);
      await page.getByRole('button', { name: 'Role to edit', exact: true }).click();
      await page.getByRole('listbox').getByRole('option', { name: 'Provider fixture role', exact: true }).click();
      check((await page.getByRole('button', { name: 'Role model', exact: true }).textContent()).includes('fixture-model'), `${cell.name}: a role retains its namespaced custom model after reload`);
      await page.getByRole('button', { name: 'Back to chat', exact: true }).click();
      await chatReady(page, cell);
      await inViewport(page, page.locator('#chat-input'), `${cell.name}: composer input remains visible and reachable`);
      await inViewport(page, page.locator('[data-model-trigger]'), `${cell.name}: model control remains visible and reachable`);
      await inViewport(page, page.getByRole('button', { name: 'Send message', exact: true }), `${cell.name}: primary send control remains visible and reachable`);
      dialog = await picker(page);
      await dialog.getByRole('textbox', { name: 'Search models', exact: true }).fill(customModel);
      await inViewport(page, dialog, `${cell.name}: model dialog stays within the viewport`);
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const popup = await dialog.boundingBox();
      const trigger = await page.locator('[data-model-trigger]').boundingBox();
      const gap = trigger.y - popup.y - popup.height;
      check(gap >= -1 && gap <= 16, `${cell.name}: filtering keeps the popup anchored immediately above the model control`);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${cell.name}: no document horizontal overflow`);
      await page.screenshot({ path: path.join(output, `${cell.name}-provider-models.png`) });
      check(state.unexpected.length === 0, `${cell.name}: no live provider or unexpected mutation requests`);
      check(!state.requests.some(request => request.endsWith('/api/agent/prompt-suggestion')), `${cell.name}: unsupported automatic suggestions do not fall back to a different provider`);
      check(state.errors.length === 0, `${cell.name}: no runtime or unexpected console errors`);
      await Promise.all(['png', 'json'].map(extension => rm(path.join(output, `${cell.name}-failure.${extension}`), { force: true })));
      checks.push({ viewport: cell, requests: state.turns, passed: true });
    } catch (error) {
      await page.screenshot({ path: path.join(output, `${cell.name}-failure.png`), fullPage: false });
      await writeFile(path.join(output, `${cell.name}-failure.json`), JSON.stringify({ error: error.stack, state }, null, 2));
      throw error;
    } finally { await context.close(); }
  }
  await writeFile(path.join(output, 'report.json'), JSON.stringify({ passed, checks, baselineConsole: [...baselineConsole] }, null, 2));
  console.log(JSON.stringify({ checks: passed.length, viewports: checks.length, mockedTurns: checks.reduce((sum, cell) => sum + cell.requests.length, 0), output, baselineConsole: [...baselineConsole] }, null, 2));
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
