// PEOPLE-19: People page timings with a large synthetic directory on the real
// WASM bundle. Measures cold list render, a search that matches one alias on
// the last page, and opening that profile. Synthetic records only; every
// write and non-loopback request is refused. A measurement, not a pass/fail
// budget: it fails only when a step does not finish.
// Run after: (cd bruhs-web && trunk build --release --dist ../target/people-ui-dist)
//   BRUHS_PEOPLE_SCALE=5000 node bruhs-web/tests/people_scale_browser.mjs
// BRUHS_PEOPLE_UI_DIST picks the bundle and BRUHS_PEOPLE_SCALE_DIR the output
// directory, so a later run never overwrites an earlier run's results.
import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_PEOPLE_UI_DIST ?? path.join(root, 'target/people-ui-dist'));
const output = path.resolve(process.env.BRUHS_PEOPLE_SCALE_DIR ?? path.join(root, 'target/people-ui-scale'));
const count = Number(process.env.BRUHS_PEOPLE_SCALE ?? 5000);
const runs = Number(process.env.BRUHS_PEOPLE_SCALE_RUNS ?? 3);
assert(Number.isInteger(count) && count >= 100, 'BRUHS_PEOPLE_SCALE must be an integer of at least 100');
await mkdir(output, { recursive: true });

let playwright;
try { playwright = await import('playwright'); } catch {
  for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
    try { playwright = await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); break; } catch {}
  }
}
assert(playwright, 'An existing Playwright installation is required');

const observation = n => ({ at_meeting_id: 'scale-meeting', kind: 'topic', text: `Synthetic topic ${n}`, created_at: 1_700_000_000_000 });
const people = Array.from({ length: count }, (_, n) => ({
  id: `scale-${String(n).padStart(5, '0')}`,
  display_name: `Scale Person ${n}`,
  aliases: n === count - 1 ? ['Needle Alias'] : [`Alias ${n}`],
  voice_embedding: null,
  observations: Array.from({ length: 4 }, (_, k) => observation(n * 4 + k)),
  tracked: true,
  created_at: 1_600_000_000_000,
  updated_at: 1_700_000_000_000 - n,
}));
const meeting = { id: 'scale-meeting', title: 'Synthetic scale meeting', started_at_ms: 1_700_000_000_000, ended_at_ms: 1_700_000_600_000, status: 'ended', calendar_event_id: null, audio_path: null, language: null, speakers: [], summary: null, thread_id: null, skip_observations: false, created_at: 1_700_000_000_000, updated_at: 1_700_000_600_000 };

const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*' });
    response.write(': fixture\n\n');
    return;
  }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) });
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]'].includes(hostname);
const samples = [];
const blocked = [];
try {
  for (let run = 0; run < runs; run += 1) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, serviceWorkers: 'block' });
    await context.route('**/*', async route => {
      const request = route.request();
      const url = new URL(request.url());
      const json = value => route.fulfill({ status: 200, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
      const api = url.pathname.startsWith('/api/') && loopback(url.hostname);
      if (api && request.method() === 'OPTIONS') return json({});
      if (api && request.method() === 'POST' && url.pathname === '/api/people/resolve-merges') return json([]);
      if (api && request.method() === 'POST' && url.pathname === '/api/people/profiles') {
        const ids = new Set(JSON.parse(request.postData() ?? '{}').ids ?? []);
        return json(people.filter(person => ids.has(person.id)));
      }
      if (!['GET', 'HEAD', 'OPTIONS'].includes(request.method())) { blocked.push(`${request.method()} ${url.pathname}`); return json({ error: 'blocked' }); }
      if (api) {
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
        if (url.pathname === '/api/people') return json(people);
        if (url.pathname === '/api/people/versions') return json({ entries: people.map(({ id, updated_at }) => ({ id, updated_at })).sort((a, b) => a.id < b.id ? -1 : 1), next_after: null });
        if (url.pathname === '/api/meetings') return json([meeting]);
        if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        if (url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        if (/\/(credentials|tasks|threads|skills|models)$/.test(url.pathname)) return json([]);
        return json({});
      }
      if (url.origin === origin) return route.continue();
      return route.abort();
    });
    const page = await context.newPage();
    page.setDefaultTimeout(60000);
    const started = Date.now();
    await page.goto(`${origin}/#/people`);
    await page.locator('#person-record-scale-00000').waitFor();
    const listMs = Date.now() - started;
    const cards = await page.locator('[id^="person-record-"]').count();
    const searchStarted = Date.now();
    await page.getByRole('searchbox', { name: 'Search people', exact: true }).fill('needle alias');
    const last = `#person-record-scale-${String(count - 1).padStart(5, '0')}`;
    await page.locator(last).waitFor();
    const searchMs = Date.now() - searchStarted;
    const openStarted = Date.now();
    await page.locator(last).click();
    await page.locator('[data-person-observation]').nth(3).waitFor();
    const profileMs = Date.now() - openStarted;
    samples.push({ run, listMs, cardsOnFirstPage: cards, searchMs, profileMs });
    await context.close();
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
const median = key => samples.map(sample => sample[key]).sort((a, b) => a - b)[Math.floor(samples.length / 2)];
const summary = { people: count, runs, medianListMs: median('listMs'), medianSearchMs: median('searchMs'), medianProfileMs: median('profileMs'), samples, blockedWrites: blocked, note: 'Cold load includes WASM start, IndexedDB caching of every synthetic profile and the server directory read. Wall-clock in headless Chrome on this machine.' };
await writeFile(path.join(output, 'results.json'), JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify(summary));
assert.equal(blocked.length, 0, 'the scale run made no writes');
