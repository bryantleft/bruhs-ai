// Synthetic-only microphone-consent UI acceptance. Never starts a real capture daemon.
// Usage: BRUHS_PLAYWRIGHT_PATH=/installed/playwright node scripts/verify-activity-monitor.playwright.mjs /absolute/isolated-dist
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile, readdir, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, extname, resolve } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
assert(process.argv[2], 'Explicit isolated same-origin build directory required');
const dist = resolve(process.argv[2]);
const artifacts = await mkdtemp(join(tmpdir(), 'bruhs-activity-monitor-'));
const report = { scope: 'Actual Settings UI with synthetic epoch/token/30-second lease daemon; no microphone, system audio, accessibility, providers, real recordings or production data. Not hardware shutdown or performance acceptance.', dist, artifacts, requests: [], commands: [], blockedExternal: [], errors: [], consoleErrors: [], consoleWarnings: [], injectedFailures: [], cases: [], screenshots: [] };
let browser, context, page, currentCase, now = Date.now();
let lease, holdGet = false, holdEnable = false, holdDisable = false, failEnable = false, failRenew = false, probeError = null;
const gates = [], sockets = new Set();
const reset = () => { lease = { epoch: randomUUID(), token: null, expires: 0 }; holdGet = holdEnable = holdDisable = failEnable = failRenew = false; probeError = null; };
reset();
const expire = () => { if (lease.token && lease.expires <= now) reset(); };
const status = () => { expire(); return { epoch: lease.epoch, enabled: !!lease.token, expires_in_ms: lease.token ? lease.expires - now : 0, probe_error: probeError }; };
const json = (res, code, body) => { res.writeHead(code, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(body)); };
const poll = async (test, message) => {
  const until = Date.now() + 8000;
  while (Date.now() < until) { if (await test()) return; await new Promise(r => setTimeout(r, 25)); }
  throw new Error(`Timed out: ${message}`);
};
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://fixture').pathname;
  report.requests.push({ case: currentCase, method: req.method, path });
  res.setHeader('Content-Security-Policy', "connect-src 'self'");
  res.setHeader('Cache-Control', 'no-store');
  if (path === '/api/capture/activity-monitor') {
    if (req.method === 'GET') {
      const snapshot = status();
      if (holdGet) { holdGet = false; gates.push(() => json(res, 200, snapshot)); return; }
      return json(res, 200, snapshot);
    }
    if (req.method !== 'POST') return json(res, 405, { error: 'Method not allowed' });
    const chunks = []; for await (const chunk of req) chunks.push(chunk);
    const command = JSON.parse(Buffer.concat(chunks).toString());
    report.commands.push({ case: currentCase, ...command, at: now });
    expire();
    if (command.epoch !== lease.epoch) return json(res, 409, { error: 'Synthetic stale epoch' });
    if (command.operation === 'enable') {
      if (failEnable) {
        report.injectedFailures.push({ case: currentCase, operation: 'enable', epoch: command.epoch, status: 409 });
        return json(res, 409, { error: 'Synthetic enable rejected' });
      }
      if (!command.token || lease.token) return json(res, 409, { error: 'Synthetic existing grant' });
      lease.token = command.token; lease.expires = now + 30000;
      const snapshot = status();
      if (holdEnable) { holdEnable = false; gates.push(() => json(res, 200, snapshot)); return; }
      return json(res, 200, snapshot);
    }
    if (command.operation === 'renew') {
      if (failRenew) {
        report.injectedFailures.push({ case: currentCase, operation: 'renew', epoch: command.epoch, status: 409 });
        return json(res, 409, { error: 'Synthetic renewal rejected' });
      }
      if (!lease.token || command.token !== lease.token) return json(res, 409, { error: 'Synthetic invalid renewal token' });
      lease.expires = now + 30000; return json(res, 200, status());
    }
    if (command.operation === 'disable') {
      const hold = holdDisable; reset(); const snapshot = status();
      if (hold) { gates.push(() => json(res, 200, snapshot)); return; }
      return json(res, 200, snapshot);
    }
    return json(res, 400, { error: 'Unknown operation' });
  }
  if (path.startsWith('/api/')) {
    if (req.method !== 'GET') return json(res, 403, { error: 'Only synthetic activity-monitor writes permitted' });
    if (path.endsWith('/stream')) {
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.write(path === '/api/events/stream' ? 'event: tasks\ndata: []\n\nevent: calendar\ndata: {"kind":"stream_opened"}\n\nevent: meetings\ndata: {"kind":"stream_opened"}\n\n' : 'data: {"kind":"stream_opened"}\n\n'); return;
    }
    const data = {
      '/api/calendar/google/state': { connected: false, email: null, last_synced_at: null, last_error: null, calendars: [] },
      '/api/capture/status': { running: false, recording: false, meeting_id: null, started_at_ms: null },
      '/api/people': [], '/api/people/versions': { entries: [], next_after: null }, '/api/meetings': [],
      '/api/calendar/google/calendars': [], '/api/terminal/sessions': { sessions: [] },
      '/api/agent/auth/claude': { authenticated: false }, '/api/agent/auth/codex': { authenticated: false },
    };
    return json(res, path in data ? 200 : 404, data[path] ?? { error: 'Unknown fixture route' });
  }
  try {
    assert.equal(req.method, 'GET');
    const file = resolve(join(dist, path === '/' ? 'index.html' : path.slice(1)));
    assert(file.startsWith(dist + '/'));
    const body = await readFile(file);
    const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
    res.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream' }); res.end(body);
  } catch { json(res, 404, { error: 'Fixture resource not found' }); }
});
server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
const toggle = () => page.getByRole('switch', { name: 'Detect meetings from audio activity', exact: true });
const master = () => page.getByRole('switch', { name: 'Auto-record calendar meetings', exact: true });
const feedback = () => page.getByTestId('activity-monitor-status');
const commands = operation => report.commands.filter(c => c.case === currentCase && (!operation || c.operation === operation));
const tick = async ms => { now += ms; await page.clock.fastForward(ms); };
const checked = async value => poll(async () => await toggle().getAttribute('aria-checked') === String(value), `switch ${value}`);
const ready = async () => {
  await toggle().waitFor({ state: 'attached', timeout: 30000 });
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Recording', exact: true }).click();
  await toggle().scrollIntoViewIfNeeded();
};
const open = async (name, legacy = false) => {
  if (context) await context.close();
  assert.equal(gates.length, 0, 'Previous request gate still held');
  currentCase = name; reset();
  context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await context.route('**/*', route => {
    const url = route.request().url();
    if (url.startsWith(report.origin + '/')) return route.continue();
    report.blockedExternal.push(url);
    return route.fulfill({ status: 200, contentType: url.includes('.css') ? 'text/css' : 'text/javascript', body: '' });
  });
  if (legacy) await context.addInitScript(() => localStorage.setItem('bruhs.auto_record', JSON.stringify({ enabled: true, min_attendees: 2, stop_delay_secs: 60, prompt_before_starting: true, audio_driven_enabled: true })));
  page = await context.newPage();
  page.on('pageerror', error => report.errors.push({ case: name, error: String(error) }));
  page.on('console', message => {
    if (message.type() === 'error') report.consoleErrors.push({ case: name, text: message.text() });
    if (message.type() === 'warning') report.consoleWarnings.push({ case: name, text: message.text() });
  });
  await page.clock.install();
  await page.goto(report.origin + '/#/settings'); await ready();
  await page.clock.pauseAt(new Date());
};
const grant = async (expectedEnables = 1) => {
  if (await master().getAttribute('aria-checked') !== 'true') await master().click();
  await toggle().click(); await checked(true);
  await poll(async () => (await feedback().innerText()).includes('on for this app session'), 'enable confirmation');
  assert(status().enabled); assert.equal(commands('enable').length, expectedEnables);
};
const pass = data => report.cases.push({ name: currentCase, passed: true, ...data });
const shot = async name => { const path = join(artifacts, name + '.png'); await page.screenshot({ path }); report.screenshots.push(path); return path; };
const releaseResponse = async method => {
  const delivered = page.waitForResponse(response => response.url().endsWith('/api/capture/activity-monitor') && response.request().method() === method && response.status() === 200);
  gates.shift()(); await (await delivered).finished();
  // Let the actual fetch continuation run before advancing its timeout clock.
  await new Promise(r => setTimeout(r, 50));
};
try {
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  report.port = server.address().port; assert(![3001,8087,8098].includes(report.port));
  report.origin = `http://127.0.0.1:${report.port}`;
  report.wasm = (await readdir(dist)).find(name => name.endsWith('.wasm'));
  report.wasmSha256 = createHash('sha256').update(await readFile(join(dist, report.wasm))).digest('hex');
  browser = await chromium.launch({ headless: true }); report.browser = browser.version();

  await open('startup-default-off-without-grant');
  await checked(false); assert.equal(await master().getAttribute('aria-checked'), 'false');
  await tick(31000); assert.equal(commands().length, 0); assert(!status().enabled); pass();

  await open('legacy-persisted-true-is-not-consent', true);
  await checked(false); await tick(31000); assert.equal(commands().length, 0);
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('bruhs.auto_record')).audio_driven_enabled), false); pass();

  await open('enable-renew-and-switch-off-stop-renewals'); await grant();
  const first = commands('enable')[0]; await tick(10001);
  await poll(() => commands('renew').length >= 1, 'lease renewal');
  assert(commands('renew').every(c => c.epoch === first.epoch && c.token === first.token));
  await toggle().click(); await checked(false); await poll(() => commands('disable').length === 1, 'disable');
  const renewals = commands('renew').length; await tick(31000); assert.equal(commands('renew').length, renewals); assert(!status().enabled); pass({ renewals });

  await open('master-off-revokes-and-does-not-restore-consent'); await grant();
  await master().click(); await checked(false); await poll(() => commands('disable').length === 1, 'master disable');
  await tick(31000); assert.equal(commands('renew').length, 0); assert(!status().enabled);
  await master().click(); await tick(10001); assert.equal(commands('enable').length, 1); await checked(false); pass();

  await open('delayed-status-after-off-cannot-enable'); await master().click(); holdGet = true; await toggle().click();
  await poll(() => gates.length === 1, 'held status GET'); await toggle().click(); await checked(false);
  await releaseResponse('GET'); await tick(10001); assert.equal(commands('enable').length, 0); assert(!status().enabled); pass();

  await open('delayed-enable-response-after-off-cannot-revive'); await master().click(); holdEnable = true; await toggle().click();
  await poll(() => gates.length === 1 && commands('enable').length === 1, 'held enable response');
  await toggle().click(); await checked(false); await poll(() => commands('disable').length === 1, 'inflight enable revoked');
  await releaseResponse('POST'); await tick(31000); await checked(false); assert.equal(commands('renew').length, 0); assert(!status().enabled); pass();

  await open('renewal-rejection-resets-switch-and-explains-failure'); await grant(); failRenew = true; await tick(10001);
  await checked(false); await poll(async () => (await feedback().innerText()).includes('Activity checks stopped:'), 'visible renewal failure');
  await poll(() => commands('disable').length === 1, 'failure cleanup');
  const error = await feedback().innerText(); assert(error.includes('409')); assert(error.includes('Turn the switch on to retry.'));
  await tick(31000); assert.equal(commands('renew').length, 1); assert(!status().enabled);
  await shot('renewal-failure-desktop'); pass({ feedback: error });

  await open('late-old-shutdown-response-cannot-erase-new-session-error'); await grant();
  holdDisable = true; await toggle().click(); await checked(false);
  await poll(() => gates.length === 1 && !status().enabled, 'old shutdown applied but response held');
  // A renewal occurs after10s, beyond the old HTTP request's5s timeout. An
  // immediate new enable409 keeps the old response genuinely in flight.
  failEnable = true; await toggle().click(); await checked(false);
  await poll(async () => (await feedback().innerText()).includes('Activity checks stopped:'), 'newer enable failure');
  await poll(() => commands('disable').length === 2, 'newer failure cleanup');
  const newerError = await feedback().innerText(); assert(newerError.includes('409'));
  await releaseResponse('POST'); await tick(10001);
  assert.equal(await feedback().innerText(), newerError, 'Old shutdown response erased newer session error');
  await checked(false); assert(!status().enabled); pass({ feedback: newerError });

  await open('probe-error-in-successful-renewal-resets-consent-with-actionable-feedback'); await grant();
  probeError = 'synthetic permission denied'; await tick(10001); await checked(false);
  await poll(async () => (await feedback().innerText()).includes('Check microphone access and your input device.'), 'actionable probe failure');
  await poll(() => commands('disable').length === 1, 'probe failure revocation');
  const probeFeedback = await feedback().innerText(); assert(probeFeedback.includes('synthetic permission denied'));
  await tick(31000); assert.equal(commands('renew').length, 1); assert(!status().enabled);
  await shot('probe-error-desktop'); pass({ feedback: probeFeedback });

  await open('reload-requires-fresh-consent-and-stops-renewals'); await grant();
  await page.reload(); await ready(); await checked(false); await tick(31000);
  assert.equal(commands('enable').length, 1); assert.equal(commands('renew').length, 0); assert(!status().enabled); pass();

  currentCase = 'mobile-permission-disclosure-wraps'; await page.setViewportSize({ width: 390, height: 844 });
  await toggle().scrollIntoViewIfNeeded();
  const copy = await page.locator('#recording').innerText();
  assert(copy.includes('Periodically checks microphone volume locally')); assert(copy.includes('Does not save audio')); assert(copy.includes('turn it on again after reopening'));
  const geometry = await page.locator('#recording').evaluate(el => ({ width: el.getBoundingClientRect().width, scrollWidth: el.scrollWidth, viewport: innerWidth }));
  assert(geometry.scrollWidth <= geometry.width + 1); await shot('mobile-permission'); pass({ geometry, copy });
  assert.equal(report.errors.length, 0);
  const injectedCases = new Set(report.injectedFailures.map(failure => failure.case));
  const expectedError = error => injectedCases.has(error.case) && /^Failed to load resource: the server responded with a status of 409 \(Conflict\)$/.test(error.text);
  report.expectedConsoleErrors = report.consoleErrors.filter(expectedError);
  report.unexpectedConsoleErrors = report.consoleErrors.filter(error => !expectedError(error));
  assert.equal(report.unexpectedConsoleErrors.length, 0, JSON.stringify(report.unexpectedConsoleErrors));
  assert(report.requests.every(r => r.method === 'GET' || (r.method === 'POST' && r.path === '/api/capture/activity-monitor')));
  report.passed = true;
} catch (error) {
  report.failure = String(error.stack || error); process.exitCode = 1;
  if (page && !page.isClosed()) { report.failureText = await page.locator('body').innerText().catch(() => ''); await shot('failure').catch(() => {}); }
} finally {
  for (const release of gates.splice(0)) release();
  if (context) await context.close(); if (browser) await browser.close();
  for (const socket of sockets) socket.destroy(); await new Promise(r => server.close(r));
  report.ownedResourcesClosed = true; console.log(JSON.stringify(report, null, 2));
}
