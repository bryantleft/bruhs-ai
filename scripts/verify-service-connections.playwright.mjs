// Synthetic-only acceptance: actual WASM UI, no provider accounts or OS keychain.
// BRUHS_PLAYWRIGHT_PATH=/installed/playwright node scripts/verify-service-connections.playwright.mjs /isolated/dist
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, extname } from 'node:path';
import { randomUUID } from 'node:crypto';
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
assert(process.argv[2], 'Explicit isolated same-origin WASM build required');
const dist = resolve(process.argv[2]);
const artifacts = await mkdtemp(join(tmpdir(), 'bruhs-service-connections-'));
const providers = ['google', 'microsoft', 'github'];
const names = { google: 'Google Workspace', microsoft: 'Microsoft 365', github: 'GitHub' };
const rows = Object.fromEntries(providers.map(provider => [provider, { id: null, provider, configured: true, account: null, scopes: ['synthetic.read'], state: 'disconnected' }]));
const attempts = new Map(), requests = [], errors = [], cases = [];
const sockets = new Set();
let origin, browser, context, heldStart, holdNextStart = false, rejectNextStart = false;
const json = (res, status, data) => { res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' }); res.end(JSON.stringify(data)); };
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://fixture').pathname;
  requests.push({ method: req.method, path });
  const start = path.match(/^\/api\/connections\/(google|microsoft|github)\/start$/);
  if (start && req.method === 'POST') {
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks).toString());
    assert.deepEqual(body, { thread_id: null, request_id: null });
    if (rejectNextStart) { rejectNextStart = false; return json(res, 503, { error: 'Synthetic sign-in unavailable' }); }
    if (holdNextStart) { holdNextStart = false; heldStart = () => json(res, 503, { error: 'Stale synthetic start failure' }); return; }
    const id = randomUUID(); attempts.set(id, { id, provider: start[1], state: 'pending', message: null });
    return json(res, 200, { id, auth_url: `${origin}/synthetic-login/${id}` });
  }
  const attempt = path.match(/^\/api\/connections\/attempts\/([^/]+)$/);
  if (attempt) {
    const saved = attempts.get(attempt[1]);
    if (!saved) return json(res, 404, { error: 'Attempt expired' });
    if (req.method === 'DELETE') { if (saved.state === 'pending') saved.state = 'cancelled'; res.writeHead(204); return res.end(); }
    return json(res, 200, saved);
  }
  const provider = path.match(/^\/api\/connections\/(google|microsoft|github)$/);
  if (provider) {
    if (req.method === 'DELETE') { Object.assign(rows[provider[1]], { id: null, account: null, state: 'disconnected' }); res.writeHead(204); return res.end(); }
    return json(res, 200, rows[provider[1]]);
  }
  if (path === '/api/connections') return json(res, 200, Object.values(rows));
  const login = path.match(/^\/synthetic-login\/([^/]+)$/);
  if (login) {
    const attempt = attempts.get(login[1]);
    assert(req.headers.cookie?.includes(`sso_${attempt.provider}=present`), 'Sign-in must reuse the existing browser profile cookie');
    res.writeHead(200, { 'content-type': 'text/html' });
    return res.end(`<h1>Synthetic browser session</h1><p>Already signed in</p><a href="/synthetic-complete/${login[1]}">Authorize service</a>`);
  }
  const complete = path.match(/^\/synthetic-complete\/([^/]+)$/);
  if (complete) {
    const attempt = attempts.get(complete[1]);
    if (attempt?.state === 'pending') {
      attempt.state = 'connected';
      Object.assign(rows[attempt.provider], { id: randomUUID(), account: `${attempt.provider}-user@example.test`, state: 'connected' });
    }
    res.writeHead(200, { 'content-type': 'text/html' }); return res.end('<p>Account connected. Return to Bruhs.</p>');
  }
  if (path.startsWith('/api/')) {
    if (req.method !== 'GET') return json(res, 403, { error: 'No agent or capture writes permitted in this fixture' });
    if (path.endsWith('/stream')) { res.writeHead(200, { 'content-type': 'text/event-stream' }); res.write('event: tasks\ndata: []\n\nevent: calendar\ndata: {"kind":"stream_opened"}\n\nevent: meetings\ndata: {"kind":"stream_opened"}\n\n'); return; }
    const data = {
      '/api/credentials': [], '/api/calendar/google/state': { connected: false, email: null, calendars: [], last_synced_at: null, last_error: null },
      '/api/capture/status': { running: false, recording: false, meeting_id: null, started_at_ms: null },
      '/api/people': [], '/api/people/versions': { entries: [], next_after: null }, '/api/meetings': [],
      '/api/calendar/google/calendars': [], '/api/terminal/sessions': { sessions: [] },
      '/api/agent/auth/claude': { authenticated: false }, '/api/agent/auth/codex': { authenticated: false },
      '/api/capture/activity-monitor': { epoch: 'synthetic', enabled: false, expires_in_ms: 0, probe_error: null },
    };
    return json(res, 200, data[path] ?? {});
  }
  try {
    const file = resolve(join(dist, path === '/' ? 'index.html' : path.slice(1)));
    assert(file.startsWith(dist + '/'));
    const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
    res.writeHead(200, { 'content-type': types[extname(file)] || 'application/octet-stream' }); res.end(await readFile(file));
  } catch { json(res, 404, { error: 'Fixture file missing' }); }
});
server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
const row = provider => context.pages()[0].locator(`[aria-label="${names[provider]} connection"]`);
const until = async (predicate, message) => {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) { if (await predicate()) return; await new Promise(resolve => setTimeout(resolve, 20)); }
  throw new Error(`Timed out: ${message}`);
};
try {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
  context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await context.addCookies(providers.map(provider => ({ name: `sso_${provider}`, value: 'present', url: origin })));
  await context.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
  const page = await context.newPage(); page.on('pageerror', error => errors.push(String(error)));
  await page.goto(origin + '/#/settings');
  await page.getByRole('navigation', { name: 'Settings sections', exact: true }).getByRole('button', { name: 'Credentials', exact: true }).click();
  for (const provider of providers) {
    await row(provider).getByRole('button', { name: 'Connect', exact: true }).waitFor();
    const popup = page.waitForEvent('popup');
    await row(provider).getByRole('button', { name: 'Connect', exact: true }).click();
    const login = await popup;
    await login.getByRole('link', { name: 'Authorize service' }).click();
    await until(async () => await row(provider).getByRole('status').innerText() === 'Connected', `${provider} connected`);
    assert.equal(await row(provider).getByText(`${provider}-user@example.test`, { exact: true }).count(), 1);
    await login.close(); cases.push(`${provider}: connect and reuse synthetic browser session`);
  }
  assert(!requests.some(request => request.path === '/api/agent/secret'), 'Connecting must never approve a command');
  await page.screenshot({ path: join(artifacts, 'connected-desktop.png') });
  await row('google').getByRole('button', { name: 'Disconnect', exact: true }).click();
  await row('google').getByRole('button', { name: 'Connect', exact: true }).waitFor();
  rejectNextStart = true;
  await row('google').getByRole('button', { name: 'Connect', exact: true }).click();
  await row('google').getByRole('alert').getByText('Synthetic sign-in unavailable', { exact: true }).waitFor();
  cases.push('visible sign-in failure with retry');
  holdNextStart = true;
  await row('google').getByRole('button', { name: 'Connect', exact: true }).click();
  await until(() => !!heldStart, 'held start A');
  await row('google').getByRole('button', { name: 'Cancel', exact: true }).click();
  const nextPopup = page.waitForEvent('popup');
  await row('google').getByRole('button', { name: 'Connect', exact: true }).click();
  const next = await nextPopup;
  await next.getByRole('link', { name: 'Authorize service' }).waitFor();
  heldStart(); heldStart = null;
  await new Promise(resolve => setTimeout(resolve, 50));
  assert.equal(await row('google').getByRole('button', { name: 'Connect', exact: true }).isDisabled(), true);
  assert.equal(await row('google').getByRole('alert').count(), 0);
  await next.getByRole('link', { name: 'Authorize service' }).click();
  await until(async () => await row('google').getByRole('status').innerText() === 'Connected', 'new attempt B survives stale A failure');
  await next.close(); cases.push('cancel and immediate reconnect ignore reordered stale failures');
  await page.setViewportSize({ width: 390, height: 844 });
  await row('github').scrollIntoViewIfNeeded();
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'No horizontal page overflow');
  const accountBounds = await row('github').getByText('github-user@example.test', { exact: true }).boundingBox();
  const rowBounds = await row('github').boundingBox();
  assert(Math.abs(accountBounds.width - rowBounds.width) <= 1, 'Mobile account details need a full row above the action buttons');
  await page.screenshot({ path: join(artifacts, 'connected-mobile.png') });
  cases.push('mobile account controls without page overflow');
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: true, cases, artifacts, scope: 'Actual WASM UI, synthetic API/OAuth only. No real SSO, keychain, or agent process.' }, null, 2));
} finally {
  heldStart?.();
  await context?.close(); await browser?.close();
  for (const socket of sockets) socket.destroy();
  await new Promise(resolve => server.close(resolve));
  await writeFile(join(artifacts, 'report.json'), JSON.stringify({ cases, errors, requests }, null, 2));
}
