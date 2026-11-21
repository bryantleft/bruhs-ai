// Opt-in live check of live meeting suggestions on the web, end to end: the
// Record button, a stand-in capture daemon streaming live transcript
// segments through the real server, a real suggestion round on a real model,
// Add task, Dismiss, the settings level, an automatic round, Stop, and durable
// task/source navigation after edits and reload. Spends
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
import { mkdtemp, mkdir, realpath, unlink, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { prepareMeetingDelivery } from './meeting_delivery_workflow.mjs';

assert(process.argv.includes('--live'), 'Pass --live to opt into real provider calls');
assert(process.argv[2] && process.argv[3], 'The workspace root and web dist are required');
const root = resolve(process.argv[2]);
const dist = resolve(process.argv[3]);
const workflowFlag = process.argv.indexOf('--delivery-workflow');
const codingEffortFlag = process.argv.indexOf('--coding-effort');
const codingEffort = codingEffortFlag >= 0 ? process.argv[codingEffortFlag + 1] : undefined;
assert(codingEffortFlag < 0 || (workflowFlag >= 0 && ['low','medium','high','xhigh','max'].includes(codingEffort)), '--coding-effort requires a delivery workflow and a supported level');
assert(workflowFlag < 0 || process.argv[workflowFlag + 1], '--delivery-workflow requires a prepared repository fixture');
const require = createRequire(import.meta.url);
const playwright = process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright';
const { chromium } = require(playwright);
const playwrightVersion = require(`${playwright}/package.json`).version;
const model = process.env.BRUHS_LIVE_MEETING_MODEL || 'claude-haiku-4-5';
assert(workflowFlag < 0 || model.startsWith('claude-'), 'This connected acceptance profile uses a claude-* meeting model and the fixture-declared Codex coding model');
const origin = 'http://127.0.0.1:43023';
const artifactParent = workflowFlag >= 0 ? join(root, 'target/connected-delivery/live') : tmpdir();
if (workflowFlag >= 0) await mkdir(artifactParent, { recursive: true });
const artifacts = await realpath(await mkdtemp(join(artifactParent, 'bruhs-meeting-suggestions-')));
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

const report = { artifacts, origin, model, checks: [], rounds: [], metadataReads: [], pageErrors: [] };
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
const daemon = { recording: false, meetingId: null, startedAt: null, streams: new Set(), sent: 0, segments: [], requests: [] };
const script = workflowFlag >= 0 ? [
  ['them', 'Huey currently dequeues SQLite work before the consumer can confirm completion, so a crashed worker can lose the task.'],
  ['me', 'I will implement durable leased delivery for Huey SQLite today, with reservation expiry, fenced acknowledgments, tests and documentation.'],
  ['them', 'Agreed. The complete reviewed contract is LEASED_DELIVERY_SPEC.md in the project, with public smoke checks in tests/test_leased_delivery.py.'],
  ['me', 'Keep the change opt-in and limited to SQLite. I will review the result before any pull request is published.'],
  ['them', 'We still need someone to review how duplicate external side effects are explained in the release notes.'],
] : [
  ['them', 'Thanks for joining. The goal today is to lock the launch plan for the pricing page.'],
  ['me', 'Dana, can you send the revised launch deck to the whole team by Thursday?'],
  ['them', 'Yes, I will send the revised deck Thursday morning.'],
  ['me', 'Great. We still have no owner for booking the venue for the offsite next week.'],
  ['them', 'Also, are we keeping the free tier for this release or not? Nobody has decided.'],
];
const later = workflowFlag >= 0 ? [
  ['me', 'The release-note review is still unassigned, and the delivery semantics must be described before release.'],
  ['them', 'Should the release notes include an example of a worker crashing after an external side effect? That remains undecided.'],
] : [
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
    confidence: 0.95,
    partial: true,
    speaker_confidence: null,
  };
  daemon.segments.push(segment);
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
  if (/^\/segments\//.test(path)) {
    if (daemon.transcriptUnavailable) return json(response, 503, { error: 'fixture capture unavailable' });
    const id = decodeURIComponent(path.slice('/segments/'.length));
    return json(response, 200, { segments: daemon.segments.filter(segment => segment.meeting_id === id).map(segment => ({ ...segment, partial: daemon.recording })) });
  }
  if (path === '/recovery' || path === '/recovery/failures') return json(response, 200, { receipts: [] });
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
  RUST_LOG: 'warn,bruhs_server=info,bruhs_core::meeting_suggestions=debug',
};
for (const name of ['BRUHS_API_URL', 'BRUHS_WEB_ORIGIN', 'ANTHROPIC_API_KEY', 'BRUHS_CLAUDE_USE_API_KEY']) delete env[name];
let deliveryWorkflow = null;

let server, browser, context, page, serverLog = '';
const shot = async name => {
  const path = join(artifacts, `${name}.png`);
  await page.screenshot({ path });
  report.screenshots = { ...(report.screenshots || {}), [name]: path };
};
const prefs = () => page.evaluate(() => JSON.parse(localStorage.getItem('bruhs.meetingSuggestions') || '{}'));
const suggestions = () => page.locator('[data-testid="meeting-suggestion"]');
const readTasks = () => page.evaluate(async () => {
  const db = await new Promise((done, fail) => { const open = indexedDB.open('bruhs'); open.onsuccess = () => done(open.result); open.onerror = fail; });
  try {
    return await new Promise((done, fail) => { const all = db.transaction('work_items').objectStore('work_items').getAll(); all.onsuccess = () => done(all.result); all.onerror = fail; });
  } finally { db.close(); }
});
const serverRunning = () => server && server.exitCode === null && server.signalCode === null;
const stopServer = async () => {
  if (!serverRunning()) return;
  server.kill('SIGTERM');
  await Promise.race([new Promise(done => server.once('exit', done)), new Promise(done => setTimeout(done, 10000))]);
  if (serverRunning()) {
    server.kill('SIGKILL');
    await new Promise(done => server.once('exit', done));
    throw new Error('The isolated server needed SIGKILL');
  }
};
const startServer = async () => {
  const logStart = serverLog.length;
  server = spawn(deliveryWorkflow?.server || join(root, 'target/debug/bruhs-server'), [], { env, cwd: deliveryWorkflow ? testHome : root, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', value => { serverLog += value; });
  server.stderr.on('data', value => { serverLog += value; });
  await waitFor(async () => {
    if (!serverRunning()) throw new Error(`Isolated server exited: ${serverLog.slice(-800)}`);
    if (!serverLog.slice(logStart).includes('bruhs-server listening on http://127.0.0.1:43023')) return false;
    return fetch(origin + '/api/health').then(response => response.ok).catch(() => false);
  }, 'isolated server', 20000);
};

try {
  if (workflowFlag >= 0) deliveryWorkflow = await prepareMeetingDelivery({ root, fixture: resolve(process.argv[workflowFlag + 1]), artifacts, home: testHome, wrapperDir, env, report, codingEffort });
  await startServer();

  browser = await chromium.launch({ headless: true });
  report.browser = { engine: 'chromium', version: browser.version(), playwright: playwrightVersion, headless: true, viewports: [[1280, 900], [390, 844]], physicalDevice: false };
  context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await deliveryWorkflow?.initialize(context);
  context.setDefaultTimeout(15000);
  page = await context.newPage();
  page.on('pageerror', error => report.pageErrors.push(String(error)));
  page.on('console', message => { if (['warning', 'error'].includes(message.type())) (report.console ||= []).push(message.text().slice(0, 300)); });
  page.on('request', request => {
    const url = new URL(request.url());
    if (request.method() === 'GET' && /^\/api\/meetings\/[^/]+$/.test(url.pathname)) report.metadataReads.push(url.pathname);
  });
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
  const addableRows = suggestions().filter({ has: page.locator('[data-testid="meeting-suggestion-add"]') });
  const addable = (deliveryWorkflow ? addableRows.filter({ hasText: /Huey|SQLite|leased delivery/i }) : addableRows).first();
  assert(await addable.count(), 'The action-item transcript must produce an addable suggestion; this acceptance case cannot be skipped.');
  {
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
    const task = await page.evaluate(async title => {
      const db = await new Promise((done, fail) => { const open = indexedDB.open('bruhs'); open.onsuccess = () => done(open.result); open.onerror = fail; });
      const rows = await new Promise((done, fail) => { const all = db.transaction('work_items').objectStore('work_items').getAll(); all.onsuccess = () => done(all.result); all.onerror = fail; });
      db.close();
      return rows.find(row => row.title === title);
    }, text);
    report.addedTask = task;
    const linked = task?.source?.kind === 'meeting' && task.source.meeting_id === daemon.meetingId;
    check('added-task-retains-meeting-identity', { passed: linked, expectedMeeting: daemon.meetingId, source: task?.source ?? null });
    if (!linked) process.exitCode = 1;
  }
  // A decision keeps focus where it was: rows are keyed, not rebuilt.
  const copy = suggestions().filter({ has: page.getByText(report.addedTask.title, { exact: true }) }).locator('[data-testid="meeting-suggestion-copy"]');
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
    await waitFor(async () => (await prefs()).calibration?.outcomes?.length === before + 1, 'Dismiss calibration to persist', 5000);
    const after = (await prefs()).calibration.outcomes;
    assert.equal(after.length, before + 1, 'Dismiss was not recorded for calibration');
    assert.equal(after.at(-1), false);
    check('dismiss-records-calibration', { outcomes: after });
  } else {
    (report.notExercised ||= []).push({ case: 'dismiss-records-calibration', reason: 'No unkept suggestion remained after adding a task.' });
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
  } else {
    (report.notExercised ||= []).push({ case: 'the-closed-pill-announces-new-suggestions', reason: 'No automatic suggestion cleared the current threshold.' });
  }
  check('automatic-round-runs-after-new-speech', { trigger: automatic.trigger, thoughts: automatic.reply.thoughts.map(t => `${t.score} ${t.kind}: ${t.text}`) });
  await shot('pill-alert-1280');

  // 7. Stop stays available and ends the meeting.
  await page.getByRole('button', { name: 'Stop recording' }).click();
  await waitFor(() => !daemon.recording, 'the daemon to stop');
  await page.locator('[data-testid="meeting-dock"]').waitFor({ state: 'detached', timeout: 15000 });
  check('stop-ends-the-meeting');

  if (deliveryWorkflow) await deliveryWorkflow.run({ page, task: report.addedTask, origin, check, waitFor, restart: async () => { await stopServer(); await startServer(); } });

  // 8. A kept task retains its source through independent edits and reload.
  const sourceId = daemon.meetingId;
  const taskId = report.addedTask.id;
  const originalSource = report.addedTask.source;
  await page.evaluate(() => { location.hash = '#/tasks'; });
  await page.getByText(report.addedTask.title, { exact: true }).first().click();
  const source = page.getByTestId('task-source-meeting');
  await source.waitFor();
  await waitFor(() => source.isEnabled(), 'the saved source meeting to resolve', 10000);
  assert.equal(await source.isEnabled(), true, 'The saved source meeting cannot be opened');
  const editedTitle = 'Prepare launch follow-up (edited)';
  const editedDescription = 'Description replaced without losing meeting context.';
  const lookupsBeforeEdits = report.metadataReads.length;
  const titleInput = page.locator('#task-detail-title');
  const descriptionInput = page.getByPlaceholder('Add description', { exact: true });
  await titleInput.fill('');
  await titleInput.pressSequentially(editedTitle, { delay: 10 });
  assert.equal(await titleInput.inputValue(), editedTitle, 'Typing lost task-title focus or characters');
  await descriptionInput.fill('');
  await descriptionInput.pressSequentially(editedDescription, { delay: 10 });
  assert.equal(await descriptionInput.inputValue(), editedDescription, 'Typing lost task-description focus or characters');
  const lookupsDuringEdits = report.metadataReads.length - lookupsBeforeEdits;
  assert(lookupsDuringEdits <= 1, 'Task edits repeatedly reloaded the source meeting');
  check('task-edits-do-not-repeat-source-lookups', { lookups: lookupsDuringEdits });
  await waitFor(async () => {
    const row = (await readTasks()).find(task => task.id === taskId);
    return row?.title === editedTitle && row.description === editedDescription;
  }, 'task edits to be durable', 10000);
  await page.reload();
  await page.getByText(editedTitle, { exact: true }).first().click();
  await source.waitFor();
  await waitFor(() => source.isEnabled(), 'the source meeting to resolve without visiting Meetings', 10000);
  const restored = (await readTasks()).find(task => task.id === taskId);
  assert.deepEqual(restored.source, originalSource);
  assert.equal(restored.description, editedDescription);
  check('source-survives-task-edits-and-reload', { id: taskId, source: restored.source });
  await shot('task-source-desktop-1280');
  await page.setViewportSize({ width: 390, height: 844 });
  await shot('task-source-touch-390');
  assert(await source.isVisible(), 'The source action is hidden at 390px');
  assert((await source.boundingBox()).height >= 44, 'The narrow source action is smaller than a 44px touch target');
  await page.setViewportSize({ width: 1280, height: 900 });

  // Use the source control's keyboard path and verify the actual transcript.
  await source.focus();
  await page.keyboard.press('Enter');
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  await page.getByText(script[0][1], { exact: true }).waitFor();
  const transcript = await page.request.get(`${origin}/api/meetings/${sourceId}/segments`);
  assert.equal(transcript.status(), 200);
  const segments = await transcript.json();
  assert.equal(segments.length, script.length + later.length);
  assert(segments.every(segment => segment.meeting_id === sourceId && !segment.partial));
  check('task-source-opens-the-exact-meeting-and-saved-transcript', { meetingId: sourceId, segments: segments.length });
  await stopServer();
  daemon.transcriptUnavailable = true;
  await startServer();
  await page.reload();
  await page.getByText(script[0][1], { exact: true }).waitFor();
  const savedTranscript = await page.request.get(`${origin}/api/meetings/${sourceId}/segments`);
  assert.equal(savedTranscript.status(), 200);
  assert.equal(savedTranscript.headers()['x-bruhs-transcript-source'], 'saved');
  assert.equal(savedTranscript.headers()['x-bruhs-transcript-refresh'], 'unavailable');
  assert.deepEqual(await savedTranscript.json(), segments);
  assert.deepEqual((await readTasks()).find(task => task.id === taskId).source, originalSource);
  check('source-and-transcript-survive-server-restart-with-capture-unavailable', { meetingId: sourceId, segments: segments.length });
  const linkedTask = page.getByTestId('meeting-linked-task').filter({ hasText: editedTitle });
  assert.equal(await linkedTask.count(), 1);
  await linkedTask.click();
  await page.waitForURL('**/#/tasks');
  assert.equal(await page.locator('#task-detail-title').inputValue(), editedTitle);
  await page.getByRole('button', { name: /Done$/, exact: false }).first().click();
  await waitFor(async () => (await readTasks()).find(task => task.id === taskId)?.status === 'Done', 'Done to be durable', 10000);
  await source.click();
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  await waitFor(async () => /Done/.test(await linkedTask.innerText()), 'meeting to show current task completion', 5000);
  check('meeting-task-navigation-and-completion-stay-connected', { id: taskId, status: 'Done' });
  await shot('meeting-linked-task-desktop-1280');

  // The source title is a display hint; identity survives a rename and a
  // different meeting using the old title. These are isolated fixture writes.
  const sameTitle = await page.request.post(`${origin}/api/meetings`, { data: { title: originalSource.title, started_at_ms: Date.now() } });
  assert.equal(sameTitle.status(), 200);
  const other = await sameTitle.json();
  assert.notEqual(other.id, sourceId);
  const renamedTitle = 'Renamed launch meeting';
  assert.equal((await page.request.patch(`${origin}/api/meetings/${sourceId}`, { data: { title: renamedTitle } })).status(), 200);
  await page.reload();
  await linkedTask.click();
  await waitFor(async () => (await source.innerText()).includes(renamedTitle), 'source label to follow the meeting rename', 5000);
  await source.click();
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  assert.equal(await page.getByRole('heading', { name: renamedTitle, exact: true }).count(), 1);
  check('source-identity-survives-renames-and-repeated-titles', { sourceId, otherId: other.id });

  // Fault injection checks the metadata lookup without extra model requests.
  const metadataUrl = `${origin}/api/meetings/${sourceId}`;
  const reopenTask = async () => {
    await page.evaluate(() => { location.hash = '#/tasks'; });
    await page.reload();
    await page.getByText(editedTitle, { exact: true }).first().click();
    await source.waitFor();
  };
  const unavailableMetadata = route => route.fulfill({ status: 503, json: { error: 'fixture metadata unavailable' } });
  await page.route(metadataUrl, unavailableMetadata);
  await reopenTask();
  await waitFor(() => source.isEnabled(), 'saved source metadata while the server lookup fails', 5000);
  await source.click();
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  await page.getByText(script[0][1], { exact: true }).waitFor();
  await page.getByText(/Transcript refresh did not finish\./).waitFor();
  check('saved-source-and-transcript-open-during-metadata-failure', { injectedStatus: 503, sourceId });
  await page.unroute(metadataUrl, unavailableMetadata);

  const missingMetadata = route => route.fulfill({ status: 404, json: { error: 'fixture metadata missing' } });
  await page.route(metadataUrl, missingMetadata);
  await reopenTask();
  await page.getByText('Source meeting is unavailable.', { exact: true }).waitFor();
  assert.equal(await source.isDisabled(), true, 'Authoritative missing metadata must disable a cached source');
  await page.unroute(metadataUrl, missingMetadata);
  const retryReply = page.waitForResponse(response => response.url() === metadataUrl && response.status() === 200);
  await page.getByRole('button', { name: 'Retry source meeting', exact: true }).click();
  await retryReply;
  await waitFor(() => source.isEnabled(), 'source lookup retry to recover', 5000);
  await source.click();
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  check('missing-source-lookup-retry-recovers-the-original-identity', { injectedStatus: 404, sourceId });

  const foreignMetadata = route => route.fulfill({ status: 200, json: other });
  await page.route(metadataUrl, foreignMetadata);
  await reopenTask();
  await waitFor(() => source.isEnabled(), 'the valid cached source while a foreign reply is rejected', 5000);
  await source.click();
  await page.waitForURL(`**/#/meetings/${sourceId}`);
  await page.getByText(/The server sent a reply this app could not read\./).waitFor();
  const foreignWasCached = await page.evaluate(async id => {
    const db = await new Promise((done, fail) => { const open = indexedDB.open('bruhs'); open.onsuccess = () => done(open.result); open.onerror = fail; });
    try {
      return await new Promise((done, fail) => { const row = db.transaction('meetings').objectStore('meetings').get(id); row.onsuccess = () => done(row.result !== undefined); row.onerror = fail; });
    } finally { db.close(); }
  }, other.id);
  assert.equal(foreignWasCached, false, 'A mismatched meeting response was cached');
  assert.equal(await page.getByRole('heading', { name: renamedTitle, exact: true }).count(), 1);
  check('foreign-meeting-replies-cannot-replace-or-populate-the-source-cache', { sourceId, rejectedId: other.id });
  await page.unroute(metadataUrl, foreignMetadata);

  // Deleting only the fixture meeting keeps the user's task and source hint.
  assert.equal((await page.request.delete(`${origin}/api/meetings/${sourceId}`)).status(), 204);
  await page.evaluate(() => { location.hash = '#/tasks'; });
  await page.reload();
  await page.getByText(editedTitle, { exact: true }).first().click();
  await source.waitFor();
  await page.getByText('Source meeting is unavailable.', { exact: true }).waitFor();
  assert.equal(await source.isDisabled(), true);
  const kept = (await readTasks()).find(task => task.id === taskId);
  assert.equal(kept.status, 'Done');
  assert.deepEqual(kept.source, originalSource);
  report.finalTask = kept;
  check('deleted-source-keeps-the-task-with-an-unavailable-source', { id: taskId, status: kept.status, source: kept.source });
  assert.deepEqual(report.pageErrors, [], 'The page threw errors');
  report.completed = true;
} catch (error) {
  report.error = String(error.stack || error);
  process.exitCode = 1;
  if (page) await shot('failure').catch(() => {});
} finally {
  if (page) report.finalURL = page.url();
  if (deliveryWorkflow && context) await context.storageState({ path: join(artifacts, 'browser-state.json'), indexedDB: true }).catch(error => { report.stateExportError = String(error); });
  await browser?.close().catch(() => {});
  await stopServer().catch(error => { report.shutdownError = String(error); process.exitCode = 1; });
  await deliveryWorkflow?.cleanup().catch(error => { report.cleanupError = String(error); process.exitCode = 1; });
  if (workflowFlag >= 0 && !deliveryWorkflow) await unlink(join(testHome, 'codex/auth.json')).catch(error => { if (error.code !== 'ENOENT') report.cleanupError = String(error); });
  for (const stream of daemon.streams) stream.end();
  capture.close();
  if (process.exitCode) report.serverLog = serverLog.slice(-3000);
  await writeFile(join(artifacts, 'server.log'), serverLog);
  report.daemonRequests = [...new Set(daemon.requests)];
  await writeFile(join(artifacts, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
