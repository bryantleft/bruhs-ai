// Real WASM/localStorage recovery, with isolated records and read-only API replies.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const [bundle, destination] = process.argv.slice(2);
assert(bundle && destination, 'Pass a built WASM dist and a fresh evidence directory');
const dist = path.resolve(bundle), output = path.resolve(destination);
const control = process.argv.includes('--control');
const touch = process.env.BRUHS_SCHEDULING_TOUCH === '1';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
await mkdir(output, { recursive: false });
const names = [
  'bruhs-web/src/state/calendar.rs', 'bruhs-web/src/state/calendar/preferences.rs',
  'bruhs-web/src/calendar_sync_stream.rs', 'bruhs-web/src/components/scheduling_status.rs',
  'bruhs-web/src/components/smart_schedule_panel.rs', 'bruhs-web/src/components/booking_page.rs',
  'bruhs-web/src/components/shell.rs', 'bruhs-web/src/components/mod.rs',
];
const sources = await Promise.all(names.map(async file => ({ file, sha256: hash(await readFile(path.join(root, file))), modifiedMs: (await stat(path.join(root, file))).mtimeMs })));
if (!control) assert((await stat(path.join(dist, 'index.html'))).mtimeMs >= Math.max(...sources.map(source => source.modifiedMs)), 'Refuse a stale candidate bundle');
const assets = await Promise.all((await readdir(dist)).filter(name => /\.(html|wasm|css|js)$/.test(name)).map(async file => ({ file, sha256: hash(await readFile(path.join(dist, file))) })));
const report = {
  scope: control ? 'Retained pre-change bundle: negative controls for startup writes only. Current source hashes do not describe that older bundle.' : 'Actual web application, browser localStorage, recovery controls and explicit edits. Synthetic records and read-only API fixtures; no model, native client or competitor run.',
  dist, output, control, touch, sources, assets,
  harnessSha256: hash(await readFile(fileURLToPath(import.meta.url))), cases: [], checks: [],
};
const streams = new Set();
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://fixture');
  if (url.pathname === '/writer') { response.writeHead(200, { 'content-type': 'text/html' }); return response.end('<!doctype html><title>Preference fixture writer</title>'); }
  if (url.pathname === '/stream') {
    response.writeHead(200, { 'content-type': 'text/event-stream', 'access-control-allow-origin': '*' });
    streams.add(response); request.on('close', () => streams.delete(response)); response.write(': fixture\n\n'); return;
  }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); return response.end(); }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'content-type': { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] || 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const browser = await chromium.launch({ headless: true });
report.browser = browser.version();
const preferences = {
  id: 'default', availability: [{ id: 'owner-hours', label: 'Owner hours', weekdays: [0, 1, 2, 3, 4, 5, 6], start_minute: 540, end_minute: 1020, enabled: true }],
  focus: { enabled: true, daily_target_minutes: 120, minimum_block_minutes: 45, preferred_start_minute: 540 }, links: [], conflicts: [], updated_at_ms: 1794844800000,
};
const raw = ' \n' + JSON.stringify(preferences, null, 2) + '\n ';
const specifications = [
  { name: 'valid-bytes', initial: raw },
  { name: 'missing-key', initial: null },
  { name: 'corrupt-startup', initial: '{unreadable-owner-hours', corrupt: true },
  { name: 'inaccessible-read', initial: raw, readFault: true },
  ...control ? [] : [
    { name: 'storage-access-denied', initial: raw, accessFault: true },
    { name: 'failed-write', initial: raw, writeFault: true },
    { name: 'failed-link', initial: raw, writeFault: true, link: true },
    { name: 'stale-tab', initial: raw, stale: true },
    { name: 'corrupted-after-startup', initial: raw, lateCorrupt: true },
    { name: 'queued-calendar-conflict', initial: raw, writeFault: true, stream: true },
  ],
];
try {
  for (const specification of specifications) {
    const current = { name: specification.name, errors: [], blocked: [], checks: [] };
    report.cases.push(current);
    const check = (name, actual, expected) => {
      const passed = JSON.stringify(actual) === JSON.stringify(expected);
      const item = { case: current.name, name, passed, actual, expected };
      report.checks.push(item); current.checks.push(item);
      if (!control) assert.equal(passed, true, `${current.name}: ${name}`);
    };
    const context = await browser.newContext({ viewport: touch ? { width: 390, height: 844 } : { width: 1280, height: 900 }, hasTouch: touch, timezoneId: 'America/Los_Angeles', serviceWorkers: 'block' });
    await context.addInitScript(({ readFault, writeFault, accessFault }) => {
      const read = Storage.prototype.getItem, write = Storage.prototype.setItem;
      const area = localStorage;
      const NativeEventSource = EventSource;
      globalThis.__calendarFrames = [];
      globalThis.__liveOpen = false;
      globalThis.EventSource = class extends NativeEventSource {
        constructor(url, options) {
          super(url, options);
          this.addEventListener('open', () => { globalThis.__liveOpen = true; });
          this.addEventListener('calendar', event => globalThis.__calendarFrames.push(JSON.parse(event.data)));
        }
      };
      globalThis.__preferenceTest = { accessFault: Boolean(accessFault) && location.pathname !== '/writer', readFault: Boolean(readFault) && location.pathname !== '/writer', writeFault: Boolean(writeFault) && location.pathname !== '/writer', writes: [] };
      globalThis.__savedPreferenceRaw = () => read.call(area, 'bruhs.scheduling');
      if (accessFault) Object.defineProperty(window, 'localStorage', { get() {
        if (globalThis.__preferenceTest.accessFault) throw new DOMException('Owned storage access denial', 'SecurityError');
        return area;
      } });
      Storage.prototype.getItem = function (key) {
        if (key === 'bruhs.scheduling' && globalThis.__preferenceTest.readFault) throw new DOMException('Owned read denial', 'SecurityError');
        return read.call(this, key);
      };
      Storage.prototype.setItem = function (key, value) {
        if (key === 'bruhs.scheduling') {
          globalThis.__preferenceTest.writes.push(value);
          if (globalThis.__preferenceTest.writeFault) throw new DOMException('Owned quota failure', 'QuotaExceededError');
        }
        return write.call(this, key, value);
      };
    }, specification);
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url());
      if (url.pathname.startsWith('/api/')) {
        if (!['GET', 'OPTIONS'].includes(request.method())) current.blocked.push({ method: request.method(), path: url.pathname });
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/stream` });
        const value = url.pathname === '/api/reactive' ? { agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] } : url.pathname === '/api/weekly-reports' ? { presets: [], reports: [], next_before: null } : /\/(meetings|people|credentials|tasks|threads|skills|models|calendars)$/.test(url.pathname) ? [] : {};
        return route.fulfill({ status: ['GET', 'OPTIONS'].includes(request.method()) ? 200 : 403, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, OPTIONS' }, body: JSON.stringify(value) });
      }
      if (url.origin === origin) return route.continue();
      current.blocked.push({ method: request.method(), url: request.url() }); return route.abort();
    });
    const writer = await context.newPage(), page = await context.newPage();
    // Same functional observation budget as the existing Today browser fixtures.
    page.setDefaultTimeout(12000);
    page.on('pageerror', error => current.errors.push(error.message));
    const saved = () => page.evaluate(() => globalThis.__savedPreferenceRaw());
    const recovery = scope => scope.getByRole('region', { name: 'Scheduling preference recovery', exact: true });
    const dismissSidebar = async () => {
      const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
      if (await scrim.isVisible()) await scrim.click({ position: { x: 375, y: 60 } });
    };
    const openControls = async () => {
      await page.evaluate(() => { location.hash = '#/calendar'; });
      await dismissSidebar();
      await page.getByRole('button', { name: 'Schedule', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: 'Scheduling controls', exact: true });
      await dialog.waitFor(); return dialog;
    };
    try {
      await writer.goto(`${origin}/writer`);
      await writer.evaluate(value => value === null ? localStorage.removeItem('bruhs.scheduling') : localStorage.setItem('bruhs.scheduling', value), specification.initial);
      await page.goto(`${origin}/#/today`);
      await page.getByRole('heading', { name: 'Today', exact: true }).waitFor();
      await dismissSidebar();
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      check('Startup preserves the exact saved bytes or missing key', await saved(), specification.initial);
      check('Startup never writes scheduling preferences', await page.evaluate(() => globalThis.__preferenceTest.writes.length), 0);
      if (control) continue;
      if (specification.stream) {
        await page.waitForFunction(async () => (await indexedDB.databases()).some(database => database.name === 'bruhs'));
        const local = {
          id: 'owner-edited-event', title: 'Locally edited commitment', start_at_ms: 1794852000000, end_at_ms: 1794855600000,
          all_day: false, color: null, notes: 'Keep this owner edit', created_at: 500, updated_at: 2000,
          source: 'google', google_event_id: 'provider-event', calendar_id: 'fixture-calendar', attendees: [],
          organizer: null, location: null, conferencing: null, recurring_event_id: null, html_link: null,
        };
        const readEvents = () => writer.evaluate(() => new Promise((resolve, reject) => {
          const request = indexedDB.open('bruhs'); request.onerror = () => reject(request.error);
          request.onsuccess = () => {
            const database = request.result, tx = database.transaction('events', 'readonly');
            const records = tx.objectStore('events').getAll();
            tx.oncomplete = () => { database.close(); resolve(records.result); };
            tx.onabort = () => { database.close(); reject(tx.error); };
          };
        }));
        await writer.evaluate(record => new Promise((resolve, reject) => {
          const request = indexedDB.open('bruhs'); request.onerror = () => reject(request.error);
          request.onsuccess = () => {
            const database = request.result, tx = database.transaction('events', 'readwrite');
            tx.objectStore('events').put(record);
            tx.oncomplete = () => { database.close(); resolve(); };
            tx.onabort = () => { database.close(); reject(tx.error); };
          };
        }), local);
        await page.reload();
        await page.getByRole('heading', { name: 'Today', exact: true }).waitFor();
        await dismissSidebar();
        await page.evaluate(() => { location.hash = '#/calendar'; });
        await page.getByText(local.title, { exact: true }).first().waitFor();
        await page.waitForFunction(() => globalThis.__liveOpen);
        for (const event of [{ kind: 'sync_completed', at_ms: 1000, upserted: 0, deleted: 0 }, { kind: 'event_upserted', event: { ...local, id: 'provider-generated-id', title: 'Changed remotely', start_at_ms: local.start_at_ms + 3600000, end_at_ms: local.end_at_ms + 3600000, updated_at: 1000 } }]) {
          for (const stream of streams) if (!stream.destroyed && !stream.writableEnded) stream.write(`event: calendar\ndata: ${JSON.stringify(event)}\n\n`);
        }
        await recovery(page).waitFor();
        check('Failed conflict write preserves saved preferences', await saved(), raw);
        check('Remote event cannot overwrite the local edit while its conflict is unsaved', await readEvents(), [local]);
        await page.evaluate(() => { globalThis.__preferenceTest.writeFault = false; });
        await recovery(page).getByRole('button', { name: 'Reload saved preferences', exact: true }).click();
        await page.waitForFunction(() => JSON.parse(globalThis.__savedPreferenceRaw()).conflicts.length === 1);
        const conflict = JSON.parse(await saved()).conflicts[0];
        check('Recovery processes the queued event exactly once', conflict.event_id, local.id);
        check('Recovered conflict keeps the owner decision pending', conflict.resolution, null);
        check('Recovery never overwrites the original local calendar event', await readEvents(), [local]);
        check('Fixture sent the remote event only once', await page.evaluate(() => globalThis.__calendarFrames.filter(event => event.kind === 'event_upserted').length), 1);
        check('No script errors', current.errors, []);
        check('No external or mutating API requests', current.blocked, []);
        continue;
      }
      if (specification.corrupt || specification.readFault || specification.accessFault) {
        await recovery(page).waitFor();
        check('Recovery names the unavailable saved policy', await recovery(page).innerText().then(text => text.includes(specification.corrupt ? 'could not be decoded' : 'could not be read')), true);
        await page.evaluate(() => { location.hash = '#/book/owner-review'; });
        await page.getByText('Booking is paused until scheduling preferences are restored.', { exact: true }).waitFor();
        check('Booking pauses while saved preferences are unavailable', await page.getByRole('button', { name: /^Book / }).count(), 0);
        await recovery(page).getByText('View saved preference data (private)', { exact: true }).click();
        const value = await recovery(page).getByRole('textbox', { name: 'Saved scheduling preference data', exact: true }).inputValue();
        check('Recovery preserves raw corruption or explicitly says the copy cannot be read', specification.corrupt ? value : value.includes('saved copy could not be read'), specification.corrupt ? specification.initial : true);
        await page.evaluate(() => { globalThis.__preferenceTest.readFault = false; globalThis.__preferenceTest.accessFault = false; });
        await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', value), raw);
        await recovery(page).getByRole('button', { name: 'Reload saved preferences', exact: true }).click();
        check('Explicit reload recovers without rewriting the saved copy', await saved(), raw);
        check('Recovery clears after a successful read', await recovery(page).count(), 0);
      }
      if (specification.lateCorrupt) await writer.evaluate(() => localStorage.setItem('bruhs.scheduling', '{later-owner-corruption'));
      const dialog = await openControls();
      if (specification.lateCorrupt) {
        await recovery(dialog).waitFor();
        check('Opening scheduling controls rechecks the saved policy', await recovery(dialog).innerText().then(text => text.includes('could not be decoded')), true);
        check('Later corruption is preserved', await saved(), '{later-owner-corruption');
        await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', value), raw);
        await recovery(dialog).getByRole('button', { name: 'Reload saved preferences', exact: true }).click();
      }
      if (specification.stale) {
        const newer = { ...preferences, availability: [{ ...preferences.availability[0], end_minute: 930 }], updated_at_ms: preferences.updated_at_ms + 1 };
        current.newer = JSON.stringify(newer);
        await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', value), current.newer);
      }
      if (specification.link) {
        await dialog.getByPlaceholder('Product review', { exact: true }).fill('Retained owner review');
        await dialog.getByRole('button', { name: 'Create', exact: true }).click();
        await recovery(dialog).waitFor();
        check('Failed link creation preserves the saved policy', await saved(), raw);
        check('A failed link is never announced as saved', await dialog.getByText('Scheduling link saved locally.', { exact: true }).count(), 0);
        await page.evaluate(() => { globalThis.__preferenceTest.writeFault = false; });
        await recovery(dialog).getByRole('button', { name: 'Reload saved preferences', exact: true }).click();
        check('Link title survives save failure and recovery', await dialog.getByPlaceholder('Product review', { exact: true }).inputValue(), 'Retained owner review');
        await dialog.getByRole('button', { name: 'Create', exact: true }).click();
        const actual = JSON.parse(await saved());
        check('Repeating link creation saves exactly one link', actual.links.map(link => link.title), ['Retained owner review']);
      } else await dialog.getByRole('button', { name: 'Start +30m', exact: true }).click();
      if ((specification.writeFault && !specification.link) || specification.stale) {
        await recovery(dialog).waitFor();
        check('Failed or stale edit leaves saved bytes unchanged', await saved(), specification.stale ? current.newer : raw);
        check('Unsafe scheduling controls pause during recovery', await dialog.getByRole('button', { name: 'Find time', exact: true }).count(), 0);
        await page.screenshot({ path: path.join(output, `${current.name}-recovery.png`) });
        await page.evaluate(() => { globalThis.__preferenceTest.writeFault = false; });
        await recovery(dialog).getByRole('button', { name: 'Reload saved preferences', exact: true }).click();
        await dialog.getByRole('button', { name: 'Start +30m', exact: true }).click();
      }
      const actual = JSON.parse(await saved());
      check('Explicit successful edit saves once without replaying a failed edit', actual.availability[0].start_minute, specification.link ? 540 : 570);
      if (specification.stale) check('Recovery preserves another tab’s changed closing time', actual.availability[0].end_minute, 930);
      await page.keyboard.press('Escape');
      await page.reload();
      await page.getByRole('button', { name: 'Schedule', exact: true }).waitFor();
      check('Reload retains the explicitly saved policy', JSON.parse(await saved()).availability[0].start_minute, specification.link ? 540 : 570);
      if (specification.link) check('Created link survives application reload', JSON.parse(await saved()).links.map(link => link.title), ['Retained owner review']);
      check('No script errors', current.errors, []);
      check('No external or mutating API requests', current.blocked, []);
      check('Page fits the viewport', await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    } catch (error) {
      current.failure = error.stack || String(error); throw error;
    } finally {
      await context.close();
    }
  }
} catch (error) {
  report.failure = error.stack || String(error);
} finally {
  report.passed = !report.failure && report.checks.every(check => check.passed);
  report.sourcesUnchanged = (await Promise.all(sources.map(async source => hash(await readFile(path.join(root, source.file))) === source.sha256))).every(Boolean);
  report.passed &&= report.sourcesUnchanged;
  await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
  await browser.close(); for (const stream of streams) stream.end(); await new Promise(resolve => server.close(resolve));
  console.log(JSON.stringify({ passed: report.passed, checks: report.checks.length, report: path.join(output, 'report.json'), failure: report.failure }));
  if (!report.passed) process.exitCode = 1;
}
