// Calendar acceptance in the real WASM app with native IndexedDB, in the
// America/Los_Angeles zone across both 2026 daylight-saving changes.
// Every record is synthetic. Google Calendar frames arrive over a local
// fixture stream; no account is used and every write to any remote fails
// the run.
// Run after: (cd bruhs-web && trunk build --dist ../target/calendar-ui-dist)
//   node bruhs-web/tests/calendar_browser.mjs
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_CALENDAR_UI_DIST ?? path.join(root, 'target/calendar-ui-dist'));
const output = path.join(root, 'target/calendar-qa');
const startedAt = new Date().toISOString();
const cells = [
  { name: 'desktop', width: 1280, height: 800, hasTouch: false },
  { name: 'compact', width: 640, height: 480, hasTouch: false },
  { name: 'touch', width: 390, height: 844, hasTouch: true },
];
const relevantSources = [
  'bruhs-web/src/components/calendar_page.rs',
  'bruhs-web/src/components/timegrid_view.rs',
  'bruhs-web/src/components/month_view.rs',
  'bruhs-web/src/components/event_create_dialog.rs',
  'bruhs-web/src/calendar_sync_stream.rs',
  'bruhs-web/src/workflow_executor.rs',
  'bruhs-web/src/clock.rs',
  'bruhs-models/src/calendar/zone.rs',
  'bruhs-models/src/models/calendar_event.rs',
  'bruhs-models/src/models/google_calendar.rs',
];
await mkdir(output, { recursive: true });
const sourceTimes = await Promise.all(relevantSources.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs, sha256: createHash('sha256').update(await readFile(path.join(root, file))).digest('hex') })));
const indexStat = await stat(path.join(dist, 'index.html'));
assert(indexStat.mtimeMs >= Math.max(...sourceTimes.map(source => source.modifiedMs)), 'Refuse stale Calendar WASM assets: rebuild target/calendar-ui-dist first');
const assetNames = (await readdir(dist)).filter(name => /\.(wasm|js|css|html)$/.test(name));
const assets = await Promise.all(assetNames.map(async name => ({ name, sha256: createHash('sha256').update(await readFile(path.join(dist, name))).digest('hex') })));
assert(assets.some(asset => asset.name.endsWith('.wasm')), 'Calendar dist must contain a compiled WASM asset');

let playwright;
try { playwright = await import('playwright'); } catch {
  for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
    try { playwright = await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); break; } catch {}
  }
}
assert(playwright, 'An existing Playwright installation is required');

const utc = iso => Date.parse(iso);
const HOUR_PX = 40;
// Synthetic fixtures. Local wall times in America/Los_Angeles.
const fallLocal = { id: 'cal-fall-local', title: 'Fall check', start: utc('2026-11-01T18:00:00Z'), end: utc('2026-11-01T19:00:00Z') }; // 10:00 PST, 11 h after midnight
const springLocal = { id: 'cal-spring-local', title: 'Spring check', start: utc('2026-03-08T17:00:00Z'), end: utc('2026-03-08T18:00:00Z') }; // 10:00 PDT, 9 h after midnight
const standup = { id: 'cal-provider-standup', google: 'g-standup', title: 'Provider standup', start: utc('2026-11-01T21:00:00Z'), end: utc('2026-11-01T21:30:00Z') }; // 13:00 PST
const holiday = { id: 'cal-provider-holiday', google: 'g-holiday', title: 'Provider holiday', start: utc('2026-11-02T00:00:00Z'), end: utc('2026-11-03T00:00:00Z') }; // date 2026-11-02
const legacy = { id: 'cal-legacy-offsite', google: 'g-legacy', title: 'Legacy offsite', start: utc('2026-11-04T00:00:00Z'), end: utc('2026-11-05T00:00:00Z') }; // stored before normalization
const syncedAt = utc('2026-11-01T19:00:00Z');

function record({ id, title, start, end, google }, extra = {}) {
  return { id, title, start_at_ms: start, end_at_ms: end, all_day: false, color: google ? null : 'blueberry', notes: null, created_at: syncedAt, updated_at: syncedAt, source: google ? 'google' : 'local', google_event_id: google ?? null, calendar_id: google ? 'cal-synthetic' : null, attendees: [], organizer: null, location: null, conferencing: null, recurring_event_id: null, html_link: google ? `https://calendar.google.com/calendar/event?eid=${google}` : null, ...extra };
}
const frames = [
  { kind: 'sync_started', at_ms: syncedAt },
  { kind: 'event_upserted', event: record(standup) },
  { kind: 'event_upserted', event: record(holiday, { all_day: true }) },
  { kind: 'sync_completed', at_ms: syncedAt, upserted: 2, deleted: 0 },
];

const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*' });
    response.write(': fixture\n\n');
    for (const frame of frames) response.write(`event: calendar\ndata: ${JSON.stringify(frame)}\n\n`);
    const timer = setInterval(() => response.write(': connected\n\n'), 1000);
    request.on('close', () => clearInterval(timer));
    return;
  }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) });
const results = { startedAt, origin, dist, timezone: 'America/Los_Angeles', assets, sourceTimes, cells: [], checks: [] };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]'].includes(hostname);

function idb(page, mode, body, arg) {
  return page.evaluate(({ mode, body, arg }) => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains('events')) { db.close(); reject(new Error('events store is missing')); return; }
      const transaction = db.transaction('events', mode);
      const store = transaction.objectStore('events');
      let result;
      if (mode === 'readonly') { const read = store.getAll(); read.onsuccess = () => { result = read.result; }; }
      else for (const value of arg) store.put(value);
      transaction.oncomplete = () => { db.close(); resolve(result ?? arg.length); };
      transaction.onabort = () => { db.close(); reject(new Error(String(transaction.error))); };
    };
  }), { mode, body, arg });
}
const readEvents = page => idb(page, 'readonly', null, null);
const seedEvents = (page, records) => idb(page, 'readwrite', null, records);

try {
  for (const cell of cells) {
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: cell.hasTouch, serviceWorkers: 'block', locale: 'en-US', timezoneId: 'America/Los_Angeles' });
    const state = { ...cell, stage: 'startup', screenshots: [], runtimeErrors: [], consoleErrors: [], unexpectedRequests: [], mockedReads: [], blockedWrites: [], layout: [] };
    results.cells.push(state);
    const check = (ok, label, evidence = undefined) => {
      const result = { cell: cell.name, stage: state.stage, label, passed: Boolean(ok), ...(evidence === undefined ? {} : { evidence }) };
      results.checks.push(result);
      if (!ok) console.error(JSON.stringify(result));
      return Boolean(ok);
    };
    await context.route('**/*', async route => {
      const request = route.request();
      const url = new URL(request.url());
      const method = request.method();
      const json = (value, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
      if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
        state.blockedWrites.push({ method, url: request.url(), body: request.postData() });
        return json({ error: 'Calendar browser verification rejects all external writes.' }, 500);
      }
      if (url.pathname.startsWith('/api/') && loopback(url.hostname)) {
        if (method === 'OPTIONS') return json({});
        state.mockedReads.push({ method, path: url.pathname });
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
        if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        if (url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        if (url.pathname === '/api/calendar/google/state') return json({ connected: true, email: null, last_synced_at: null, last_error: null, calendars: [] });
        if (/\/(meetings|people|credentials|tasks|threads|skills|models|calendars)$/.test(url.pathname)) return json([]);
        return json({});
      }
      if (url.origin === origin) return route.continue();
      state.unexpectedRequests.push({ method, url: request.url() });
      return route.abort();
    });
    const page = await context.newPage();
    page.setDefaultTimeout(10000);
    page.on('pageerror', error => state.runtimeErrors.push(error.stack ?? error.message));
    page.on('console', message => { if (message.type() === 'error') state.consoleErrors.push(message.text()); });
    const main = page.locator('#app-main');
    const viewGroup = () => main.getByRole('group', { name: 'Calendar view', exact: true });
    const modeButton = name => viewGroup().getByRole('button', { name, exact: true });
    const block = id => page.locator(`[role="button"][data-event-id="${id}"]`);
    const column = () => page.locator('div.relative.border-l').first();
    const dialog = () => page.getByRole('dialog', { name: 'Calendar event', exact: true });
    async function snapshot(name) {
      const file = `${cell.name}-${name}.png`;
      await page.screenshot({ path: path.join(output, file) });
      state.screenshots.push(file);
    }
    async function dismissSidebar() {
      const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
      if (await scrim.isVisible()) {
        await scrim.click({ position: { x: cell.width - 12, y: 60 } });
        await page.getByRole('complementary', { name: 'Sidebar navigation', exact: true }).waitFor({ state: 'hidden' });
      }
    }
    async function ready() {
      await viewGroup().waitFor();
      await dismissSidebar();
    }
    async function persisted(predicate, label) {
      const deadline = Date.now() + 8000;
      let records = [];
      do {
        records = await readEvents(page);
        if (predicate(records)) { check(true, label, { count: records.length }); return records; }
        await sleep(80);
      } while (Date.now() < deadline);
      check(false, label, { records });
      return records;
    }
    async function target(locator, label, minimum) {
      await locator.scrollIntoViewIfNeeded();
      const box = await locator.boundingBox();
      const need = cell.hasTouch ? 43.9 : minimum - 0.1;
      check(box && box.height >= need && box.width >= Math.min(need, box.width), `${label} keeps a ${cell.hasTouch ? 44 : minimum}px target`, box);
      const hit = await locator.evaluate(element => { const r = element.getBoundingClientRect(); const at = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return Boolean(at && (at === element || element.contains(at))); });
      check(hit, `${label} is reachable without an overlay`);
    }
    async function topOf(id) {
      return block(id).evaluate(element => Number.parseFloat(element.style.top));
    }
    async function scrollGridTo(px) {
      await column().evaluate((element, px) => { element.parentElement.parentElement.scrollTop = px; }, px);
    }
    // HTML5 drag with explicit client coordinates, so rows outside the
    // viewport are reachable in the compact cell. `payload` overrides the
    // data the app's dragstart writes (to try a forged move).
    async function drag(id, rowPx, payload = null) {
      return page.evaluate(({ id, rowPx, payload }) => {
        const source = document.querySelector(`[role="button"][data-event-id="${id}"]`);
        const column = source.closest('div.relative.border-l');
        const data = new DataTransfer();
        const top = source.getBoundingClientRect().top;
        const start = new DragEvent('dragstart', { bubbles: true, cancelable: true, dataTransfer: data, clientY: top + 5 });
        const started = source.dispatchEvent(start);
        if (payload !== null) data.setData('text/plain', payload);
        const clientY = column.getBoundingClientRect().top + rowPx + 5;
        column.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer: data, clientY }));
        column.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: data, clientY }));
        return { started, payload: data.getData('text/plain') };
      }, { id, rowPx, payload });
    }
    async function noOverflow(label) {
      const evidence = await page.evaluate(() => ({ width: document.documentElement.clientWidth, scrollWidth: document.documentElement.scrollWidth }));
      state.layout.push({ label, ...evidence });
      check(evidence.scrollWidth <= evidence.width + 1, `${label} has no horizontal page overflow`, evidence);
    }
    try {
      // Fall-back day: Sunday 2026-11-01, 11:30 PST.
      await page.clock.setFixedTime(new Date('2026-11-01T19:30:00Z'));
      await page.goto(`${origin}/#/calendar`);
      await ready();
      state.stage = 'seed';
      await seedEvents(page, [record(fallLocal), record(springLocal), record(legacy, { all_day: true })]);
      await page.reload();
      await ready();
      check(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone) === 'America/Los_Angeles', 'browser runs in the America/Los_Angeles zone');

      state.stage = 'provider sync';
      const synced = await persisted(records => {
        const h = records.find(r => r.google_event_id === 'g-holiday');
        const l = records.find(r => r.id === legacy.id);
        return h && l && records.some(r => r.google_event_id === 'g-standup') && h.start_at_ms === utc('2026-11-02T08:00:00Z') && l.start_at_ms === utc('2026-11-04T08:00:00Z');
      }, 'Google all-day dates (streamed and previously stored) are saved at local midnight of their own date');
      const storedHoliday = synced.find(r => r.google_event_id === 'g-holiday');
      check(storedHoliday && storedHoliday.end_at_ms === utc('2026-11-03T08:00:00Z') && storedHoliday.all_day, 'Google all-day end stays exclusive at the next local midnight', storedHoliday);
      check(synced.filter(r => r.google_event_id === 'g-standup').length === 1, 'a replayed provider frame does not duplicate the event');

      state.stage = 'view navigation';
      for (const name of ['Day', 'Week', 'Month']) {
        await modeButton(name).click();
        check(await modeButton(name).getAttribute('aria-pressed') === 'true', `${name} view is selectable and announces its selected state`);
        await target(modeButton(name), `${name} view switch`, 28);
      }
      for (const name of ['Previous', 'Next']) await target(main.getByRole('button', { name, exact: true }), `${name} navigation`, 32);
      await target(main.getByRole('button', { name: 'Today', exact: true }), 'Today navigation', 32);
      await target(main.getByRole('button', { name: 'New event', exact: true }), 'New event', 32);
      const selectedClass = await modeButton('Month').getAttribute('class');
      check(selectedClass.includes('blueberry') && !selectedClass.includes('persimmon'), 'the selected view uses blueberry selection, not brand persimmon', selectedClass);

      state.stage = 'month all-day';
      const holidayPill = page.locator(`button[data-event-id="${holiday.id}"]`);
      await holidayPill.waitFor();
      const holidayDays = await page.locator(`button[data-event-id="${holiday.id}"]`).evaluateAll(pills => pills.map(pill => pill.parentElement.firstElementChild.textContent.trim()));
      check(holidayDays.length === 1 && holidayDays[0] === '2', 'the Google all-day event shows on November 2 only', holidayDays);
      const legacyDays = await page.locator(`button[data-event-id="${legacy.id}"]`).evaluateAll(pills => pills.map(pill => pill.parentElement.firstElementChild.textContent.trim()));
      check(legacyDays.length === 1 && legacyDays[0] === '4', 'the migrated all-day event shows on November 4 only', legacyDays);
      check(await holidayPill.getAttribute('aria-label') === 'Open Google event Provider holiday, read only', 'month pills name the event and its read-only source');
      await noOverflow('month view');
      await snapshot('month');

      state.stage = 'fall-back day grid';
      await main.getByRole('button', { name: 'Today', exact: true }).click();
      await modeButton('Day').click();
      await block(fallLocal.id).waitFor();
      const fallTop = await topOf(fallLocal.id);
      check(Math.abs(fallTop - 10 * HOUR_PX) < 0.5, '10:00 on the 25-hour day sits on the 10:00 row', { fallTop });
      const standupTop = await topOf(standup.id);
      check(Math.abs(standupTop - 13 * HOUR_PX) < 0.5, 'a 13:00 Google event on the 25-hour day sits on the 13:00 row', { standupTop });
      check(await block(standup.id).getAttribute('draggable') === 'false' && await block(standup.id).getAttribute('data-read-only') === 'true', 'the Google event is marked read only and is not draggable');
      check(await block(standup.id).locator('.cursor-ns-resize').count() === 0 && await block(fallLocal.id).locator('.cursor-ns-resize').count() === 1, 'only the local event offers a resize handle');
      await noOverflow('day view');
      await snapshot('fall-day');

      state.stage = 'read-only provider';
      const providerDrag = await drag(standup.id, 16 * HOUR_PX);
      check(!providerDrag.started && providerDrag.payload === '', 'a Google event refuses to start a drag', providerDrag);
      await drag(standup.id, 16 * HOUR_PX, `move:${standup.id}|5.00`);
      await sleep(300);
      const afterDrag = (await readEvents(page)).find(r => r.id === standup.id);
      check(afterDrag && afterDrag.start_at_ms === standup.start && afterDrag.end_at_ms === standup.end, 'a forged drop cannot move a Google event', afterDrag);
      await block(standup.id).click();
      await dialog().waitFor();
      await dialog().getByText('Google event (read only)', { exact: true }).waitFor();
      check(await dialog().getByRole('textbox').first().isDisabled(), 'the read-only Google dialog disables the title');
      check(await dialog().getByRole('button', { name: 'Save', exact: true }).isDisabled(), 'the read-only Google dialog cannot save');
      check(await dialog().getByRole('link', { name: 'Open in Google Calendar', exact: true }).count() === 1, 'the read-only Google dialog links to the provider instead');
      check(await dialog().getByRole('button', { name: 'Delete', exact: true }).count() === 0, 'a Google event offers no local delete');
      await snapshot('provider-dialog');
      await target(dialog().getByRole('button', { name: 'Cancel', exact: true }), 'event dialog Cancel', 32);
      await target(dialog().getByRole('button', { name: 'Save', exact: true }), 'event dialog Save', 32);
      await dialog().getByRole('button', { name: 'Cancel', exact: true }).click();
      await dialog().waitFor({ state: 'hidden' });

      state.stage = 'local edit on the fall-back day';
      const localDrag = await drag(fallLocal.id, 14 * HOUR_PX);
      check(localDrag.started && localDrag.payload.startsWith(`move:${fallLocal.id}|`), 'a local event starts a drag', localDrag);
      const moved = await persisted(records => records.find(r => r.id === fallLocal.id)?.start_at_ms === utc('2026-11-01T22:00:00Z'), 'dragging the local event to the 14:00 row saves 14:00 PST');
      const movedRecord = moved.find(r => r.id === fallLocal.id);
      check(movedRecord && movedRecord.end_at_ms - movedRecord.start_at_ms === 3_600_000, 'the move keeps the one-hour duration', movedRecord);
      await block(fallLocal.id).focus();
      await page.keyboard.press('Enter');
      await dialog().getByText('Edit event', { exact: true }).waitFor();
      check(true, 'Enter on a focused event block opens its editor');
      await dialog().getByRole('button', { name: 'Cancel', exact: true }).click();
      await dialog().waitFor({ state: 'hidden' });

      state.stage = 'spring-forward day grid';
      await page.clock.setFixedTime(new Date('2026-03-08T19:00:00Z'));
      await main.getByRole('button', { name: 'Today', exact: true }).click();
      await block(springLocal.id).waitFor();
      const springTop = await topOf(springLocal.id);
      check(Math.abs(springTop - 10 * HOUR_PX) < 0.5, '10:00 on the 23-hour day sits on the 10:00 row', { springTop });
      await scrollGridTo(8 * HOUR_PX);
      const box = await column().boundingBox();
      const scrollTop = await column().evaluate(element => element.parentElement.parentElement.scrollTop);
      await page.mouse.click(box.x + box.width - 12, box.y + 11 * HOUR_PX + 12);
      await dialog().getByText('New event', { exact: true }).waitFor();
      const startLabel = await dialog().locator('label', { hasText: 'Start' }).locator('xpath=..').locator('button').innerText();
      check(startLabel.trim() === '2026-03-08 11:00', 'clicking the 11:00 row on the 23-hour day prefills 11:00', { startLabel, scrollTop });
      await snapshot('spring-create');
      await dialog().getByRole('button', { name: 'Cancel', exact: true }).click();
      await noOverflow('spring day view');

      state.stage = 'restart';
      await page.reload();
      await ready();
      const afterReload = await readEvents(page);
      check(afterReload.find(r => r.id === fallLocal.id)?.start_at_ms === utc('2026-11-01T22:00:00Z') && afterReload.find(r => r.id === legacy.id)?.start_at_ms === utc('2026-11-04T08:00:00Z'), 'moves and migrations survive a reload unchanged', afterReload.map(r => [r.id, r.start_at_ms]));
      check(afterReload.length === 5, 'reload keeps exactly the five synthetic events', afterReload.length);
    } catch (error) {
      check(false, 'workflow completed without an unexpected browser interaction failure', { error: error.stack ?? String(error) });
      await snapshot('failure').catch(() => {});
    } finally {
      state.stage = 'runtime and privacy';
      check(state.runtimeErrors.length === 0, 'no JavaScript/WASM runtime errors', state.runtimeErrors);
      check(state.blockedWrites.length === 0, 'calendar flows attempt no remote writes', state.blockedWrites);
      check(state.unexpectedRequests.length === 0, 'no unexpected external requests', state.unexpectedRequests);
      await context.close();
      console.log(JSON.stringify({ cell: cell.name, passed: results.checks.filter(check => check.cell === cell.name && check.passed).length, failed: results.checks.filter(check => check.cell === cell.name && !check.passed).length, screenshots: state.screenshots }));
      await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2) + '\n');
    }
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
results.finishedAt = new Date().toISOString();
results.summary = { passed: results.checks.filter(check => check.passed).length, failed: results.checks.filter(check => !check.passed).length, cells: results.cells.length, screenshots: results.cells.reduce((count, cell) => count + cell.screenshots.length, 0) };
await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2) + '\n');
console.log(JSON.stringify({ ...results.summary, artifacts: output }));
if (results.summary.failed) process.exitCode = 1;
