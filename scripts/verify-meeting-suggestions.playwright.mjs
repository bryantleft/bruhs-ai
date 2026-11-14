// Opt-in live check of live meeting suggestions on the web, end to end: the
// Record button, a stand-in capture daemon streaming live transcript
// segments through the real server, a real suggestion round on a real model,
// Add task, Dismiss, the settings level, an automatic round and Stop. Spends
// real tokens, so it is not in the gate.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43023 trunk build --dist /tmp/bruhs-meeting-suggestions-dist)
//   BRUHS_PLAYWRIGHT_PATH=/path/to/playwright \
//     node scripts/verify-meeting-suggestions.playwright.mjs "$PWD" /tmp/bruhs-meeting-suggestions-dist --live
//
// The app runs with a disposable HOME, database and browser profile. The
// provider CLI keeps the real HOME so its sign-in works.
// BRUHS_LIVE_MEETING_MODEL picks the model (default claude-haiku-4-5).
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { mkdtemp, mkdir, realpath, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';

assert(process.argv.includes('--live'), 'Pass --live to opt into real provider calls');
assert(process.argv[2] && process.argv[3], 'The workspace root and web dist are required');
const root = resolve(process.argv[2]);
const dist = resolve(process.argv[3]);
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const model = process.env.BRUHS_LIVE_MEETING_MODEL || 'claude-haiku-4-5';
const origin = 'http://127.0.0.1:43023';
const artifacts = await realpath(await mkdtemp(join(tmpdir(), 'bruhs-meeting-suggestions-')));
const testHome = join(artifacts, 'home');
await mkdir(testHome, { recursive: true, mode: 0o700 });

const wrapperDir = join(artifacts, 'bin');
await mkdir(wrapperDir, { mode: 0o700 });
const shellQuote = value => `'${value.replaceAll("'", "'\\''")}'`;
for (const provider of ['claude', 'codex']) {
  const installed = spawnSync('/usr/bin/which', [provider], { encoding: 'utf8' }).stdout.trim();
  if (!installed.startsWith('/')) continue;
  await writeFile(join(wrapperDir, provider), `#!/bin/sh\nexport HOME=${shellQuote(homedir())}\nexec ${shellQuote(installed)} "$@"\n`, { mode: 0o700 });
}

const report = { artifacts, origin, model, checks: [], rounds: [], pageErrors: [] };
const check = (name, detail = {}) => {
  report.checks.push({ name, passed: true, ...detail });
  console.error(JSON.stringify({ check: name, ...detail }));
};
const waitFor = async (test, description, timeout = 60000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await test()) return;
    await new Promise(done => setTimeout(done, 250));
  }
  throw new Error(`Timed out: ${description}`);
};

// ── Stand-in capture daemon ────────────────────────────────────────────────
const token = randomBytes(32).toString('hex');
const tokenFile = join(artifacts, 'capture-token');
await writeFile(tokenFile, token, { mode: 0o600 });
const daemon = { recording: false, meetingId: null, startedAt: null, streams: new Set(), sent: 0, requests: [] };
const script = [
  ['them', 'Thanks for joining. The goal today is to lock the launch plan for the pricing page.'],
  ['me', 'Dana, can you send the revised launch deck to the whole team by Thursday?'],
  ['them', 'Yes, I will send the revised deck Thursday morning.'],
  ['me', 'Great. We still have no owner for booking the venue for the offsite next week.'],
  ['them', 'Also, are we keeping the free tier for this release or not? Nobody has decided.'],
];
const later = [
  ['me', 'One more thing: Priya needs the final budget numbers before the board review on Monday.'],
  ['them', 'I can pull the budget numbers together, but I need the vendor quotes first.'],
];
const send = (speaker, text) => {
  const index = daemon.sent++;
  const segment = {
    id: `live-${index}`,
    meeting_id: daemon.meetingId,
    speaker_id: speaker,
    start_ms: index * 5000,
    end_ms: index * 5000 + 4500,
    text,
    confidence: null,
    partial: true,
    speaker_confidence: null,
  };
  const frame = `data: ${JSON.stringify({ kind: 'segment', meeting_id: daemon.meetingId, segment })}\n\n`;
  for (const stream of daemon.streams) stream.write(frame);
};
const json = (response, status, body) => {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(body));
};
const capture = createServer(async (request, response) => {
  const path = new URL(request.url, 'http://capture').pathname;
  daemon.requests.push(`${request.method} ${path}`);
  if (path === '/health') return json(response, 200, { ok: true });
  if (request.headers.authorization !== `Bearer ${token}`) return json(response, 401, { error: 'token' });
  if (path === '/status') {
    return json(response, 200, {
      recording: daemon.recording,
      meeting_id: daemon.meetingId,
      started_at_ms: daemon.startedAt,
      system_audio_permission: 'granted',
      system_audio_capturing: false,
    });
  }
  if (path === '/events') {
    response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
    response.write(': open\n\n');
    daemon.streams.add(response);
    request.on('close', () => daemon.streams.delete(response));
    return undefined;
  }
  if (request.method === 'POST' && path === '/capture/start') {
    let body = '';
    for await (const chunk of request) body += chunk;
    const start = JSON.parse(body || '{}');
    Object.assign(daemon, { recording: true, meetingId: start.meeting_id, startedAt: start.started_at_ms ?? Date.now() });
    return json(response, 200, { meeting_id: start.meeting_id, wav_path: join(artifacts, 'meeting.wav') });
  }
  if (request.method === 'POST' && path === '/capture/stop') {
    const id = daemon.meetingId;
    Object.assign(daemon, { recording: false });
    return json(response, 200, { meeting_id: id, ended_at_ms: Date.now(), wav_path: join(artifacts, 'meeting.wav') });
  }
  if (/^\/segments\//.test(path)) return json(response, 200, { segments: [] });
  return json(response, 404, {});
});
await new Promise(done => capture.listen(0, '127.0.0.1', done));

const env = {
  ...process.env,
  HOME: testHome,
  PATH: `${wrapperDir}:${process.env.PATH}`,
  BRUHS_SERVER_PORT: '43023',
  BRUHS_WEB_DIST: dist,
  BRUHS_MEETING_DB: join(artifacts, 'meetings.sqlite3'),
  BRUHS_CAPTURE_URL: `http://127.0.0.1:${capture.address().port}`,
  BRUHS_CAPTURE_TOKEN_FILE: tokenFile,
  RUST_LOG: 'warn,bruhs_core::meeting_suggestions=debug',
};
for (const name of ['BRUHS_API_URL', 'BRUHS_WEB_ORIGIN', 'ANTHROPIC_API_KEY', 'BRUHS_CLAUDE_USE_API_KEY']) delete env[name];

let server, browser, page, serverLog = '';
const shot = async name => {
  const path = join(artifacts, `${name}.png`);
  await page.screenshot({ path });
  report.screenshots = { ...(report.screenshots || {}), [name]: path };
};
const prefs = () => page.evaluate(() => JSON.parse(localStorage.getItem('bruhs.meetingSuggestions') || '{}'));
const suggestions = () => page.locator('[data-testid="meeting-suggestion"]');

try {
  server = spawn(join(root, 'target/debug/bruhs-server'), [], { env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', value => { serverLog += value; });
  server.stderr.on('data', value => { serverLog += value; });
  await waitFor(async () => {
    if (server.exitCode !== null) throw new Error(`Isolated server exited: ${serverLog.slice(-800)}`);
    return fetch(origin + '/api/health').then(response => response.ok).catch(() => false);
  }, 'isolated server', 20000);

  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  context.setDefaultTimeout(15000);
  page = await context.newPage();
  page.on('pageerror', error => report.pageErrors.push(String(error)));
  page.on('console', message => { if (['warning', 'error'].includes(message.type())) (report.console ||= []).push(message.text().slice(0, 300)); });
  page.on('response', async response => {
    if (!response.url().endsWith('/api/agent/meeting-suggestions')) return;
    const request = JSON.parse(response.request().postData() || '{}');
    let body = null;
    try { body = await response.json(); } catch { /* not JSON */ }
    report.rounds.push({ status: response.status(), trigger: request.trigger, model: request.model, lines: request.transcript?.length, context: request.context?.map(item => item.reference), reply: body });
    console.error(JSON.stringify({ round: report.rounds.length, status: response.status(), trigger: request.trigger, thoughts: body?.thoughts?.map(t => `${t.score} ${t.kind}: ${t.text}`) }));
  });

  // 1. The default level answers only when asked; the settings row shows it.
  await page.goto(origin + '/#/settings');
  const level = page.getByRole('button', { name: /Meeting suggestions/ }).first();
  await level.waitFor({ timeout: 30000 });
  assert.match(await level.innerText(), /When I ask/, 'The default level is not When I ask');
  check('settings-default-is-when-i-ask');

  // 2. Record through the real UI; the stand-in daemon starts.
  await page.goto(origin + '/#/meetings');
  report.captureStatus = await (await fetch(origin + '/api/capture/status', { headers: { Origin: origin } })).text();
  const record = page.getByRole('button', { name: /^Record$/ });
  await waitFor(() => record.isEnabled(), 'Record to enable once the daemon answers');
  await record.click();
  await waitFor(() => daemon.recording, 'the daemon to start recording');
  await page.locator('[data-testid="meeting-dock"]').waitFor();
  check('record-starts-the-live-dock', { meeting: daemon.meetingId });

  // 3. Live segments reach the dock through the server.
  await waitFor(() => daemon.streams.size > 0, 'the server to subscribe to daemon events');
  for (const [speaker, text] of script) send(speaker, text);
  await page.getByRole('button', { name: 'Expand meeting dock' }).click();
  await waitFor(async () => (await page.locator('[data-testid="live-transcript-chunk"]').count()) >= script.length, 'live segments in the dock');
  await page.locator('[data-testid="meeting-suggestions"]').waitFor();
  assert.equal(report.rounds.length, 0, 'A round ran before it was asked for');
  check('live-segments-reach-the-dock-and-nothing-runs-unasked');

  // 4. Suggest now runs one real round and shows what clears the threshold.
  await page.locator('[data-testid="meeting-suggestions-now"]').click();
  await waitFor(() => report.rounds.length === 1, 'the suggestion round to return', 90000);
  const first = report.rounds[0];
  assert.equal(first.status, 200, `The round failed: ${JSON.stringify(first.reply).slice(0, 400)}`);
  assert.equal(first.trigger, 'request');
  assert.equal(first.model, model);
  assert(first.reply.thoughts.length > 0, 'The model returned no thoughts for a transcript with clear action items');
  const shown = first.reply.thoughts.filter(thought => thought.score >= 4).length;
  await waitFor(async () => (await suggestions().count()) === Math.min(shown, 3), `${shown} suggestions in the dock`, 10000);
  assert(shown > 0, `No thought cleared the default threshold: ${JSON.stringify(first.reply.thoughts)}`);
  check('suggest-now-runs-a-real-round', { thoughts: first.reply.thoughts.map(t => `${t.score} ${t.kind}: ${t.text}`), shown });
  await shot('dock-desktop-1280');
  await page.setViewportSize({ width: 390, height: 844 });
  await shot('dock-touch-390');
  await page.setViewportSize({ width: 1280, height: 900 });

  // 5. Add task saves a real task; Dismiss tunes the threshold.
  const addable = suggestions().filter({ has: page.locator('[data-testid="meeting-suggestion-add"]') }).first();
  if (await addable.count()) {
    const text = (await addable.locator('p').nth(1).innerText()).trim();
    await addable.locator('[data-testid="meeting-suggestion-add"]').click();
    await page.getByText('Task added', { exact: false }).first().waitFor();
    await waitFor(async () => page.evaluate(async title => {
      const db = await new Promise((done, fail) => { const open = indexedDB.open('bruhs'); open.onsuccess = () => done(open.result); open.onerror = fail; });
      const rows = await new Promise((done, fail) => { const all = db.transaction('work_items').objectStore('work_items').getAll(); all.onsuccess = () => done(all.result); all.onerror = fail; });
      db.close();
      return rows.some(row => JSON.stringify(row).includes(title));
    }, text), 'the added task in IndexedDB', 10000);
    check('add-task-saves-a-task', { title: text });
  } else {
    report.skippedAdd = 'No action item or follow up cleared the threshold in this round.';
  }
  // A decision keeps focus where it was: rows are keyed, not rebuilt.
  const copy = page.locator('[data-testid="meeting-suggestion-copy"]').first();
  await copy.focus();
  await page.keyboard.press('Enter');
  await waitFor(async () => page.evaluate(() => document.activeElement?.dataset?.testid === 'meeting-suggestion-copy'), 'focus to stay on Copy', 3000);
  await page.waitForTimeout(500);
  assert.equal(await page.evaluate(() => document.activeElement?.dataset?.testid), 'meeting-suggestion-copy', 'Copy lost keyboard focus');
  check('copy-keeps-keyboard-focus');
  const before = (await prefs()).calibration?.outcomes?.length ?? 0;
  const dismissable = page.locator('[data-testid="meeting-suggestion-dismiss"]').first();
  if (await dismissable.count()) {
    const count = await suggestions().count();
    await dismissable.click();
    await waitFor(async () => (await suggestions().count()) === count - 1, 'the dismissed suggestion to leave', 5000);
    const after = (await prefs()).calibration.outcomes;
    assert.equal(after.length, before + 1, 'Dismiss was not recorded for calibration');
    assert.equal(after.at(-1), false);
    check('dismiss-records-calibration', { outcomes: after });
  }

  // 6. Switching to Automatic with alerts in Settings starts rounds without asking.
  // Hash navigation keeps the page, and so the live session, loaded.
  await page.evaluate(() => { location.hash = '#/settings'; });
  await page.getByRole('button', { name: /Meeting suggestions/ }).first().click();
  await page.getByRole('option', { name: 'Automatic with alerts' }).click();
  await waitFor(async () => (await prefs()).proactivity === 'alert', 'the alert level to be saved', 5000);
  check('settings-saves-the-alert-level');
  await page.evaluate(() => { location.hash = '#/meetings'; });
  await page.locator('[data-testid="meeting-dock"]').waitFor();
  // A closed dock is where the alert level announces new suggestions; an
  // open one shows them and marks them seen.
  const collapse = page.getByRole('button', { name: 'Collapse meeting dock' });
  if (await collapse.isVisible()) await collapse.click();
  for (const [speaker, text] of later) send(speaker, text);
  await waitFor(() => report.rounds.length >= 2, 'an automatic round after the pause', 90000);
  const automatic = report.rounds[1];
  assert(['pause', 'speech'].includes(automatic.trigger), `The second round was not automatic: ${automatic.trigger}`);
  assert.equal(automatic.status, 200, `The automatic round failed: ${JSON.stringify(automatic.reply).slice(0, 300)}`);
  const threshold = (await prefs()).calibration?.threshold ?? 4;
  const fresh = automatic.reply.thoughts.filter(thought => thought.score >= threshold);
  if (fresh.length) {
    await page.locator('[data-testid="meeting-dock-unseen"]').waitFor({ timeout: 10000 });
    const status = await page.locator('[data-testid="meeting-dock-status"]').textContent();
    assert.match(status, /new suggestion/, 'The status does not announce the new suggestions to screen readers');
    check('the-closed-pill-announces-new-suggestions', { badge: await page.locator('[data-testid="meeting-dock-unseen"]').innerText(), status });
  }
  check('automatic-round-runs-after-new-speech', { trigger: automatic.trigger, thoughts: automatic.reply.thoughts.map(t => `${t.score} ${t.kind}: ${t.text}`) });
  await shot('pill-alert-1280');

  // 7. Stop stays available and ends the meeting.
  await page.getByRole('button', { name: 'Stop recording' }).click();
  await waitFor(() => !daemon.recording, 'the daemon to stop');
  await page.locator('[data-testid="meeting-dock"]').waitFor({ state: 'detached', timeout: 15000 });
  check('stop-ends-the-meeting');
  assert.deepEqual(report.pageErrors, [], 'The page threw errors');
} catch (error) {
  report.error = String(error.stack || error);
  process.exitCode = 1;
  if (page) await shot('failure').catch(() => {});
} finally {
  await browser?.close().catch(() => {});
  if (server && server.exitCode === null) {
    server.kill('SIGTERM');
    await Promise.race([new Promise(done => server.once('exit', done)), new Promise(done => setTimeout(done, 10000))]);
    if (server.exitCode === null) server.kill('SIGKILL');
  }
  for (const stream of daemon.streams) stream.end();
  capture.close();
  if (process.exitCode) report.serverLog = serverLog.slice(-3000);
  report.daemonRequests = [...new Set(daemon.requests)];
  await writeFile(join(artifacts, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
