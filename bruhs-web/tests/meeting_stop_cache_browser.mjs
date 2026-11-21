// Real WASM/server/SQLite; scripted capture transport and no model calls.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, stat, writeFile } from 'node:fs/promises';
import { createHash, randomBytes } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
assert(process.argv[2] && process.argv[3], 'Pass a fresh web dist and companion-server executable');
const root = process.cwd(), dist = path.resolve(process.argv[2]), binary = path.resolve(process.argv[3]);
assert((await stat(path.join(dist, 'index.html'))).mtimeMs >= (await stat('bruhs-web/src/components/meeting_live.rs')).mtimeMs, 'Build the current WASM before testing');
const parent = path.join(root, 'target/connected-delivery/stop-cache'); await mkdir(parent, { recursive: true }); const out = await mkdtemp(path.join(parent, 'run-'));
const home = path.join(out, 'home'); await mkdir(home);
const report = { scope: 'Manual web Stop, immediate navigation away, then host restart with capture unavailable. Real WASM/API/SQLite and synthetic speech transport. No model, physical audio or native-client acceptance claim.', checks: [], errors: [], output: out, serverSha256: createHash('sha256').update(await readFile(binary)).digest('hex') };
const driver = await readFile(fileURLToPath(import.meta.url)); report.harnessSha256 = createHash('sha256').update(driver).digest('hex'); await writeFile(path.join(out, 'driver.mjs'), driver);
const check = (passed, label, actual) => { report.checks.push({ label, passed: Boolean(passed), actual }); console.log(JSON.stringify({ label, passed: Boolean(passed) })); assert(passed, label); };
const wait = async (test, label) => { const end = Date.now() + 60000; while (Date.now() < end) { const value = await test(); if (value) return value; await new Promise(done => setTimeout(done, 100)); } throw new Error('Timed out: ' + label); };
const token = randomBytes(32).toString('hex'), tokenFile = path.join(out, 'capture-token'); await writeFile(tokenFile, token, { mode: 0o600 });
const state = { recording: false, id: null, streams: new Set(), segments: [], unavailable: false, requests: [] };
const json = (response, status, value) => { response.writeHead(status, { 'Content-Type': 'application/json' }); response.end(JSON.stringify(value)); };
const capture = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://capture'), route = url.pathname; state.requests.push({ route, recording: state.recording });
  if (route === '/health') return json(response, 200, { ok: true });
  if (request.headers.authorization !== 'Bearer ' + token) return json(response, 401, { error: 'capture auth' });
  if (route === '/status') return json(response, 200, { recording: state.recording, meeting_id: state.id, started_at_ms: state.recording ? Date.now() : null, system_audio_permission: 'granted', system_audio_capturing: false });
  if (route === '/events') { response.writeHead(200, { 'Content-Type': 'text/event-stream' }); response.write(': open\n\n'); state.streams.add(response); request.on('close', () => state.streams.delete(response)); return; }
  if (route === '/capture/start') { let body = ''; for await (const bytes of request) body += bytes; const value = JSON.parse(body); state.id = value.meeting_id; state.recording = true; return json(response, 200, { meeting_id: state.id, wav_path: path.join(out, 'synthetic.wav') }); }
  if (route === '/capture/stop') { state.recording = false; return json(response, 200, { meeting_id: state.id, ended_at_ms: Date.now(), wav_path: path.join(out, 'synthetic.wav') }); }
  if (route.startsWith('/segments/')) return json(response, state.unavailable ? 503 : 200, state.unavailable ? { error: 'capture offline' } : { segments: state.segments.map(segment => ({ ...segment, partial: state.recording })) });
  if (route.startsWith('/recovery')) return json(response, 200, { receipts: [] });
  return json(response, 404, {});
});
await new Promise(done => capture.listen(0, '127.0.0.1', done));
const base = 'http://127.0.0.1:43023';
const env = { HOME: home, XDG_DATA_HOME: path.join(home, 'data'), XDG_CONFIG_HOME: path.join(home, 'config'), CODEX_HOME: path.join(home, 'codex'), CLAUDE_CONFIG_DIR: path.join(home, 'claude'), PATH: '/usr/bin:/bin', BRUHS_SERVER_PORT: '43023', BRUHS_WEB_DIST: dist, BRUHS_WORKBENCH_DIR: path.join(home, 'workbench'), BRUHS_DELIVERY_DIR: path.join(home, 'delivery'), BRUHS_MEETING_DB: path.join(home, 'meetings.sqlite3'), BRUHS_MAIL_DB: path.join(home, 'mail.sqlite3'), BRUHS_REACTIVE_DB: path.join(home, 'reactive.sqlite3'), BRUHS_CAPTURE_URL: `http://127.0.0.1:${capture.address().port}`, BRUHS_CAPTURE_TOKEN_FILE: tokenFile, RUST_LOG: 'bruhs_server=info' };
let server, browser, page, log = '';
async function start() { const offset = log.length; server = spawn(binary, [], { env, cwd: home, stdio: ['ignore', 'pipe', 'pipe'] }); server.stdout.on('data', bytes => { log += bytes; }); server.stderr.on('data', bytes => { log += bytes; }); await wait(async () => { if (server.exitCode !== null) throw new Error('Owned server exited'); return log.slice(offset).includes('bruhs-server listening on http://127.0.0.1:43023') && (await fetch(base + '/api/coordination')).ok; }, 'owned server'); }
async function stop() { if (!server || server.exitCode !== null || server.signalCode !== null) return; server.kill('SIGTERM'); await wait(() => server.exitCode !== null || server.signalCode !== null, 'owned server stop'); }
try {
  await start(); const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH); browser = await chromium.launch({ headless: true });
  report.browser = { engine: 'chromium', version: browser.version(), headless: true, physicalDevice: false };
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' }); page = await context.newPage(); page.on('pageerror', error => report.errors.push(String(error)));
  await page.goto(base + '/#/meetings'); const record = page.getByRole('button', { name: 'Record', exact: true }); await wait(() => record.isEnabled(), 'Record enabled'); await record.click(); await wait(() => state.recording && state.streams.size, 'recording with live stream');
  const speech = ['We agreed to retain the stopped transcript.', 'Its source must stay available after task work.', 'The backend may go offline before the meeting is opened.'];
  speech.forEach((text, index) => { const segment = { id: `speech-${index}`, meeting_id: state.id, speaker_id: 'me', start_ms: index * 5000, end_ms: index * 5000 + 4500, text, confidence: 0.95, partial: true, speaker_confidence: null }; state.segments.push(segment); for (const stream of state.streams) stream.write(`data: ${JSON.stringify({ kind: 'segment', meeting_id: state.id, segment })}\n\n`); });
  const stoppedSnapshot = page.waitForResponse(async response => {
    if (new URL(response.url()).pathname !== `/api/meetings/${state.id}/segments` || !response.ok()) return false;
    const rows = await response.json(); return rows.length === speech.length && rows.every(row => !row.partial);
  }, { timeout: 60000 }).then(response => ({ response }), error => ({ error }));
  await page.getByRole('button', { name: 'Stop recording', exact: true }).click(); await page.getByTestId('meeting-dock').waitFor({ state: 'detached' });
  await page.evaluate(() => { location.hash = '#/tasks'; });
  const stopped = await stoppedSnapshot; if (stopped.error) throw stopped.error; await stopped.response.finished();
  check(state.requests.some(request => request.route === `/segments/${state.id}` && !request.recording), 'Stop refreshes the transcript without opening the meeting');
  state.unavailable = true; await stop(); await start();
  const response = await page.request.get(`${base}/api/meetings/${state.id}/segments`), saved = await response.json();
  check(response.status() === 200 && response.headers()['x-bruhs-transcript-source'] === 'saved' && response.headers()['x-bruhs-transcript-refresh'] === 'unavailable' && saved.length === speech.length && saved.every(row => !row.partial), 'restart serves the stopped snapshot while capture is unavailable', saved);
  await page.goto(base + '/#/meetings/' + state.id); await page.getByText(speech[0], { exact: true }).waitFor(); await page.getByText('Showing the saved transcript. Capture refresh is unavailable; retry when connected.', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(out, 'desktop.png') }); await page.setViewportSize({ width: 390, height: 844 }); await page.screenshot({ path: path.join(out, 'touch.png') });
  check(await page.locator('html').evaluate(element => element.scrollWidth <= innerWidth), 'saved transcript fits the narrow viewport'); check(report.errors.length === 0, 'no browser runtime errors', report.errors); report.completed = true;
} catch (error) { report.error = String(error.stack || error); process.exitCode = 1; if (page) await page.screenshot({ path: path.join(out, 'failure.png') }).catch(() => {}); }
finally { await browser?.close(); await stop(); for (const stream of state.streams) stream.end(); capture.close(); report.requests = state.requests; await writeFile(path.join(out, 'server.log'), log); await writeFile(path.join(out, 'report.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify({ completed: report.completed || false, checks: report.checks.length, error: report.error, output: out })); }
