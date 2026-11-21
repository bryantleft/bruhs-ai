// Mounted Today clock and review invalidation on actual WASM/IndexedDB.
// Synthetic Date advance plus real focus/visibility events; natural OS resume
// and background timer scheduling are outside this suite. No external writes.
// Synthetic records only; no build, install, provider or native-app acceptance.
//   node bruhs-web/tests/today_clock_browser.mjs /fresh/gate/dist
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

async function findRoot(directory) {
  for (;;) {
    try { await stat(path.join(directory, 'bruhs-web/src/components/today_page.rs')); return directory; } catch {}
    const parent = path.dirname(directory);
    assert(parent !== directory, 'Cannot find the bruhs workspace');
    directory = parent;
  }
}
const self = fileURLToPath(import.meta.url);
const root = await findRoot(path.dirname(self));
assert(process.argv[2], 'Explicit fresh WASM dist required; do not use the old gate dist');
const dist = path.resolve(process.argv[2]);
const evidenceRoot = path.join(root, 'target/pillar-completion-qa');
const startedAt = new Date().toISOString();
const output = path.join(evidenceRoot, 'today-browser', startedAt.replace(/[:.]/g, '-') + '-clock');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const sourceFiles = [
  'bruhs-web/src/components/today_page.rs', 'bruhs-web/src/storage.rs',
  'bruhs-web/src/storage/day_plans.rs', 'bruhs-web/src/task_saves.rs',
  'bruhs-web/src/task_saves/planning.rs', 'bruhs-web/src/task_saves/queue.rs',
  'bruhs-web/src/task_saves/queue/planning.rs',
  'bruhs-models/src/models/planning/day_plan.rs',
  'bruhs-web/src/storage/tasks.rs', 'bruhs-web/src/workflow_executor.rs',
  'bruhs-web/src/clock.rs', 'bruhs-web/src/ui/dialog.rs',
  'bruhs-web/src/ui/button.rs', 'bruhs-web/index.html', 'bruhs-web/tailwind.css',
  'bruhs-models/src/models/planning.rs', 'bruhs-models/src/models/work_item.rs',
  'bruhs-models/src/models/calendar_event.rs', 'bruhs-models/src/models/attendee.rs',
];
const sources = await Promise.all(sourceFiles.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs, sha256: digest(await readFile(path.join(root, file))) })));
const auditPath = path.join(evidenceRoot, 'today-source-audit.json');
const probePath = path.join(evidenceRoot, 'planning-clock-probe.json');
async function priorJson(file) {
  try { const bytes = await readFile(file); return { path: file, sha256: digest(bytes), value: JSON.parse(bytes) }; }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}
const priorAudit = await priorJson(auditPath), priorClock = await priorJson(probePath);
const audit = priorAudit?.value ?? { sourceSha256: {} };
const index = await stat(path.join(dist, 'index.html'));
const newestSourceMs = Math.max(...sources.map(source => source.modifiedMs));
assert(index.mtimeMs >= newestSourceMs, 'Refuse stale Today assets; wait for the coordinated successful gate bundle');
const assetNames = (await readdir(dist)).filter(name => /\.(wasm|js|css|html)$/.test(name));
assert(assetNames.some(name => name.endsWith('.wasm')), 'Supplied dist has no WASM asset');
const assets = await Promise.all(assetNames.map(async name => ({ name, sha256: digest(await readFile(path.join(dist, name))) })));
const report = {
  startedAt, dist, output,
  scope: 'Actual WASM mounted Today and real isolated IndexedDB. Synthetic Date is advanced after an open review, then real focus/visibility listeners refresh the app clock; actual browser timers and IDB requests remain native. Verifies new-day invalidation and review/apply/Undo across three viewports.',
  limits: ['No native/desktop acceptance', 'No provider, account, email, keychain, or real user data', 'No live-stream or cross-tab in-memory reconciliation', 'No delayed/failed persistence injection', 'No claim that the source audit or arithmetic probe itself established application acceptance'],
  freshness: { indexModifiedMs: index.mtimeMs, newestSourceMs }, sources, assets,
  harnessSha256: digest(await readFile(self)),
  priorEvidence: { audit: priorAudit && { path: priorAudit.path, sha256: priorAudit.sha256 }, clockProbe: priorClock && { path: priorClock.path, sha256: priorClock.sha256 }, sourceMatchesAudit: sources.filter(source => audit.sourceSha256[source.file]).map(source => ({ file: source.file, matches: source.sha256 === audit.sourceSha256[source.file], current: source.sha256, audited: audit.sourceSha256[source.file] })), clockProbeTimeZone: priorClock?.value.timeZone ?? null },
  cases: [], checks: [], artifactManifest: 'artifact-sha256.json',
};
await mkdir(output, { recursive: true });
let playwright;
if (process.env.BRUHS_PLAYWRIGHT_PATH) playwright = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH);
else {
  try { playwright = await import('playwright'); } catch {
    for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
      try { playwright = await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); break; } catch {}
    }
  }
}
assert(playwright, 'Use an existing Playwright installation; no installation is performed');
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (!['GET', 'HEAD'].includes(request.method)) { response.writeHead(403); response.end(); return; }
  if (url.pathname === '/__today-storage-writer') { response.writeHead(200, { 'Content-Type': 'text/html' }); response.end('<!doctype html><title>Synthetic Today storage writer</title><p>Isolated IndexedDB fixture writer; no app, provider, or external service.</p>'); return; }
  if (url.pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*' });
    response.write(': isolated fixture\n\n');
    const timer = setInterval(() => response.write(': connected\n\n'), 1000);
    request.on('close', () => clearInterval(timer));
    return;
  }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
report.origin = origin;
const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const normalize = value => Array.isArray(value) ? value.map(normalize) : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, normalize(value[key])])) : value;
const equal = (left, right) => JSON.stringify(normalize(left)) === JSON.stringify(normalize(right));
const taskContent = tasks => tasks.map(({ updated_at, ...item }) => item);
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]'].includes(hostname);

async function readDB(page) {
  const result = await page.evaluate(() => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('The actual app must create its schema before the fixture reads it')); };
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      if (!['work_items', 'events'].every(store => db.objectStoreNames.contains(store))) { db.close(); reject(new Error('Actual app schema is missing task/event stores')); return; }
      const tx = db.transaction(['work_items', 'events'], 'readonly');
      const tasks = tx.objectStore('work_items').getAll(), events = tx.objectStore('events').getAll();
      const schema = { name: db.name, version: db.version, stores: [...db.objectStoreNames], taskKeyPath: tx.objectStore('work_items').keyPath, eventKeyPath: tx.objectStore('events').keyPath };
      tx.oncomplete = () => { db.close(); resolve({ schema, tasks: tasks.result, events: events.result }); };
      tx.onabort = () => { db.close(); reject(new Error(String(tx.error))); };
    };
  }));
  result.tasks.sort((a, b) => a.id.localeCompare(b.id));
  result.events.sort((a, b) => a.id.localeCompare(b.id));
  return result;
}
async function writeDB(page, writes) {
  await page.evaluate(writes => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('Fixture refuses to invent a schema')); };
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result, tx = db.transaction(['work_items', 'events'], 'readwrite');
      for (const [store, records] of Object.entries(writes.puts ?? {})) for (const record of records) tx.objectStore(store).put(record);
      for (const [store, ids] of Object.entries(writes.deletes ?? {})) for (const id of ids) tx.objectStore(store).delete(id);
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onabort = () => { db.close(); reject(new Error(String(tx.error))); };
    };
  }), writes);
}
function task(id, now, date, minutes = 45) {
  return { id, project_id: null, title: `Synthetic focus ${id} — 東京`, description: 'Synthetic baseline description', status: 'Todo', priority: 'High', due_date: date, planning: { cycle: null, dependency_ids: [], estimate_minutes: minutes, recurrence: null, time_block_event_ids: [], waiting_on: null }, created_at: now - 60_000, updated_at: now - 60_000 };
}
function event(id, title, start, end, now) {
  return { id, title, start_at_ms: start, end_at_ms: end, all_day: false, color: 'blueberry', notes: 'Synthetic independently owned calendar record', created_at: now, updated_at: now, source: 'local', google_event_id: null, calendar_id: null, attendees: [], organizer: null, location: null, conferencing: null, recurring_event_id: null, html_link: null };
}
const cells = [{ name: 'desktop', width: 1280, height: 800, hasTouch: false }, { name: 'compact', width: 640, height: 480, hasTouch: false }, { name: 'touch', width: 390, height: 844, hasTouch: true }];
const semanticCases = ['mounted-rollover'];
const specifications = cells.flatMap(cell => semanticCases.map(kind => ({ id: `${cell.name}-${kind}`, kind, cell, date: '2026-11-16', timezoneId: 'America/Los_Angeles', now: '2026-11-16T16:00:00Z', expectedHour: 8 })));

try {
  for (const spec of specifications) {
    const state = { ...spec, phase: 'setup', screenshots: [], snapshots: [], requests: [], blockedWrites: [], unexpected: [], runtimeErrors: [], consoleErrors: [] };
    report.cases.push(state);
    const check = (passed, label, expected = undefined, actual = undefined) => {
      const result = { case: spec.id, phase: state.phase, label, passed: Boolean(passed), ...(expected === undefined ? {} : { expected }), ...(actual === undefined ? {} : { actual }) };
      report.checks.push(result);
      if (!passed) console.error(JSON.stringify({ case: spec.id, phase: state.phase, label, passed: false, evidence: path.join(output, 'report.json') }));
      return Boolean(passed);
    };
    const context = await browser.newContext({ viewport: { width: spec.cell.width, height: spec.cell.height }, hasTouch: spec.cell.hasTouch, timezoneId: spec.timezoneId, locale: 'en-US', serviceWorkers: 'block' });
    await context.addInitScript(({ now }) => {
      globalThis.__todayQaNow = now;
      const NativeDate = Date;
      globalThis.Date = new Proxy(NativeDate, {
        construct(target, args) { return Reflect.construct(target, args.length ? args : [globalThis.__todayQaNow]); },
        apply() { return new NativeDate(globalThis.__todayQaNow).toString(); },
        get(target, key) { return key === 'now' ? () => globalThis.__todayQaNow : Reflect.get(target, key); },
      });
      // Passive completion evidence: native transactions, results, and timing
      // are unchanged. Writer-page observations are separate from app writes.
      globalThis.__todayQaTransactions = [];
      const transaction = IDBDatabase.prototype.transaction;
      IDBDatabase.prototype.transaction = function (...args) {
        const tx = Reflect.apply(transaction, this, args);
        const entry = { sequence: globalThis.__todayQaTransactions.length, stores: [...tx.objectStoreNames], mode: tx.mode, state: 'pending' };
        globalThis.__todayQaTransactions.push(entry);
        for (const name of ['complete', 'abort']) tx.addEventListener(name, () => { entry.state = name; entry.error = tx.error?.name ?? null; });
        return tx;
      };
    }, { now: Date.parse(spec.now) });
    context.on('page', page => {
      page.on('pageerror', error => state.runtimeErrors.push(error.stack ?? error.message));
      page.on('console', message => { if (message.type() === 'error') state.consoleErrors.push(message.text()); });
    });
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url()), method = request.method();
      const json = (value, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS', 'Access-Control-Allow-Private-Network': 'true', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
      if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) { state.blockedWrites.push({ method, url: request.url(), body: request.postData() }); return json({ error: 'Today QA blocks every external mutation' }, 403); }
      if (url.pathname.startsWith('/api/') && loopback(url.hostname)) {
        state.requests.push({ method, url: request.url() });
        if (method === 'OPTIONS') return json({});
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
        if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        if (url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        if (url.pathname === '/api/calendar/google/state') return json({ connected: false, email: null, calendars: [], last_synced_at: null, last_error: null });
        if (/\/(meetings|people|credentials|tasks|threads|skills|models|calendars)$/.test(url.pathname)) return json([]);
        return json({});
      }
      if (url.origin === origin) return route.continue();
      state.unexpected.push({ method, url: request.url() });
      return route.abort();
    });
    const page = await context.newPage(), writer = await context.newPage();
    page.setDefaultTimeout(12000);
    const today = page.locator('section[aria-labelledby="today-title"]');
    const review = page.getByRole('dialog', { name: 'Review day plan', exact: true });
    const undo = today.getByRole('button', { name: 'Undo', exact: true });
    async function screenshot(label) { const file = `${spec.id}-${label}.png`; await page.screenshot({ path: path.join(output, file) }); state.screenshots.push(file); }
    async function capture(label) {
      const data = await readDB(writer);
      state.snapshots.push({ label, ...data, primaryTransactions: await page.evaluate(() => globalThis.__todayQaTransactions) });
      return data;
    }
    async function dismissSidebar() {
      const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
      if (await scrim.isVisible()) await scrim.click({ position: { x: spec.cell.width - 12, y: 60 } });
    }
    async function cursor() { return page.evaluate(() => globalThis.__todayQaTransactions.length); }
    async function mutationOutcome(sequence) {
      const deadline = Date.now() + 12000;
      do {
        const transactions = await page.evaluate(sequence => globalThis.__todayQaTransactions.filter(tx => tx.sequence >= sequence && tx.mode === 'readwrite' && tx.stores.includes('work_items') && tx.stores.includes('events') && tx.state !== 'pending'), sequence);
        if (transactions.length) return { kind: 'transaction', transactions };
        const alerts = await today.getByRole('alert').allTextContents();
        const dialogAlerts = await review.getByRole('alert').allTextContents();
        if (alerts.length || dialogAlerts.length) return { kind: 'rejected', alerts: [...alerts, ...dialogAlerts] };
        if (await review.isVisible() && await review.getByRole('button', { name: 'Apply plan', exact: true }).isDisabled()) return { kind: 'disabled-review', text: await review.innerText() };
        await sleep(50);
      } while (Date.now() < deadline);
      return { kind: 'timeout', transactions: await page.evaluate(() => globalThis.__todayQaTransactions) };
    }
    async function apply() {
      const sequence = await cursor();
      const button = review.getByRole('button', { name: 'Apply plan', exact: true });
      if (await button.isEnabled()) await button.click();
      const outcome = await mutationOutcome(sequence);
      state.snapshots.push({ label: 'apply-outcome', outcome });
      check(outcome.kind !== 'timeout', 'Apply reaches a durable transaction or explicit blocked state', 'transaction/rejected/disabled-review', outcome);
      return outcome;
    }
    async function undoApplied() {
      await undo.waitFor();
      const sequence = await cursor();
      await undo.click();
      const outcome = await mutationOutcome(sequence);
      state.snapshots.push({ label: 'undo-outcome', outcome });
      check(outcome.kind !== 'timeout', 'Undo reaches a durable transaction or explicit refusal', 'transaction/rejected', outcome);
      return outcome;
    }
    async function controlGeometry(locator, label) {
      await locator.scrollIntoViewIfNeeded();
      const actual = await locator.evaluate(element => {
        const box = element.getBoundingClientRect(), x = Math.max(0, Math.min(innerWidth - 1, box.x + box.width / 2)), y = Math.max(0, Math.min(innerHeight - 1, box.y + box.height / 2));
        const hit = document.elementFromPoint(x, y);
        return { x: box.x, y: box.y, width: box.width, height: box.height, hit: Boolean(hit && (hit === element || element.contains(hit))), documentWidth: document.documentElement.clientWidth, documentScrollWidth: document.documentElement.scrollWidth };
      });
      check(actual.hit && actual.x >= -1 && actual.x + actual.width <= spec.cell.width + 1 && actual.documentScrollWidth <= actual.documentWidth + 1, `${label} is scroll-reachable without horizontal overflow`, { viewportWidth: spec.cell.width, hit: true }, actual);
      if (spec.cell.hasTouch) check(actual.width >= 43.9 && actual.height >= 43.9, `${label} has a 44px touch target`, { minimumWidth: 44, minimumHeight: 44 }, actual);
    }
    try {
      await page.goto(`${origin}/#/today`);
      await today.getByRole('heading', { name: 'Today', exact: true }).waitFor();
      await dismissSidebar();
      await page.waitForFunction(async () => (await indexedDB.databases()).some(db => db.name === 'bruhs' && db.version >= 13));
      await writer.goto(`${origin}/__today-storage-writer`);
      const initial = await capture('app-created-empty-schema');
      check(initial.schema.name === 'bruhs' && initial.schema.taskKeyPath === 'id' && initial.schema.eventKeyPath === 'id', 'fixture uses the actual app-created task/event schema', { database: 'bruhs', taskKeyPath: 'id', eventKeyPath: 'id' }, initial.schema);
      check(initial.tasks.length === 0 && initial.events.length === 0, 'isolated browser context starts with no task/event data', { tasks: [], events: [] }, { tasks: initial.tasks, events: initial.events });
      const now = Date.parse(spec.now), mainId = `${spec.id}-task-0`, doneId = `${spec.id}-completed-dependency`;
      const tasks = spec.kind === 'workday-boundaries' ? Array.from({ length: 6 }, (_, index) => task(`${spec.id}-task-${index}`, now, spec.date, 90)) : [task(mainId, now, spec.date), { ...task(doneId, now, spec.date), status: 'Done' }];
      // This boundary fixture intentionally uses a saved nine-hour day so all
      // six 90-minute tasks fit. Default hours and weekends have their own suite.
      if (spec.kind === 'workday-boundaries') await writer.evaluate(now => localStorage.setItem('bruhs.scheduling', JSON.stringify({ id: 'default', availability: [{ id: 'clock-boundary', label: 'Boundary fixture', weekdays: [1, 2, 3, 4, 5], start_minute: 540, end_minute: 1080, enabled: true }], focus: { enabled: true, daily_target_minutes: 120, minimum_block_minutes: 45, preferred_start_minute: 540 }, links: [], conflicts: [], updated_at_ms: now })), now);
      await writeDB(writer, { puts: { work_items: tasks } });
      await page.reload();
      await today.getByRole('heading', { name: 'Today', exact: true }).waitFor();
      await dismissSidebar();
      await page.locator('section[aria-labelledby="focus-heading"]').getByText(tasks[0].title, { exact: true }).waitFor();
      const wall = await page.evaluate(({ date }) => {
        const [year, month, day] = date.split('-').map(Number), now = new Date();
        return { now: Date.now(), hour: now.getHours(), date: `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`, start: new Date(year, month - 1, day, 9).getTime(), end: new Date(year, month - 1, day, 18).getTime(), label: now.toLocaleDateString('en-US', { weekday: 'long', month: 'long', day: 'numeric' }) };
      }, { date: spec.date });
      check(wall.now === now && wall.hour === spec.expectedHour && wall.date === spec.date, 'WASM page clock is fixed before the local workday', { instant: now, hour: spec.expectedHour, date: spec.date, timezoneId: spec.timezoneId }, wall);
      const actualDateLabel = await page.locator('#today-title').locator('..').locator('p').innerText();
      check(actualDateLabel === wall.label, 'Today renders the fixed browser-local calendar date', wall.label, actualDateLabel);
      const before = await capture('loaded-before-review');
      state.phase = 'proposal review';
      await controlGeometry(today.getByRole('button', { name: 'Plan my day', exact: true }), 'Plan my day');
      await today.getByRole('button', { name: 'Plan my day', exact: true }).click();
      await review.waitFor();
      await review.getByText(tasks[0].title, { exact: true }).waitFor();
      state.reviewText = await review.innerText();
      check(await review.getByRole('button', { name: 'Apply plan', exact: true }).isEnabled(), 'synthetic ready work produces an actionable reviewed proposal', true, state.reviewText);
      await controlGeometry(review.getByRole('button', { name: 'Apply plan', exact: true }), 'Apply plan');
      await controlGeometry(review.getByRole('button', { name: 'Cancel', exact: true }), 'Review Cancel');
      await screenshot('review');

      if (spec.kind === 'mounted-rollover') {
        state.phase = 'mounted date rollover';
        const next = now + 24 * 3_600_000;
        await page.evaluate(value => {
          globalThis.__todayQaNow = value;
          document.dispatchEvent(new Event('visibilitychange'));
          window.dispatchEvent(new FocusEvent('focus'));
        }, next);
        await page.waitForFunction(() => document.querySelector('#today-title')?.parentElement.querySelector('p')?.textContent === 'Tuesday, November 17');
        await review.waitFor({ state: 'hidden' });
        check(await page.locator('#today-title').locator('..').locator('p').innerText() === 'Tuesday, November 17', 'mounted Today refreshes its local date after resume/visibility', 'Tuesday, November 17');
        check(!await review.isVisible(), 'rollover invalidates an open prior-day review instead of applying its blocks', false, await review.isVisible());
        const unchanged = await capture('after-rollover-before-new-review');
        check(equal(unchanged.tasks, before.tasks) && equal(unchanged.events, before.events), 'date refresh and review invalidation write no task or event data', before, unchanged);
        check((await today.getByRole('alert').allTextContents()).some(text => /date|timezone|review/i.test(text)), 'stale date review gives actionable feedback');
        await screenshot('rollover-refused-old-review');
        await today.getByRole('button', { name: 'Plan my day', exact: true }).click();
        await review.waitFor();
        await apply();
        await undo.waitFor();
        const refreshed = await capture('new-day-applied');
        const expectedStart = await page.evaluate(() => new Date(2026, 10, 17, 9).getTime());
        check(refreshed.events.length === 1 && refreshed.events[0].start_at_ms === expectedStart, 'new review uses the new local workday rather than mount-time bounds', expectedStart, refreshed.events);
        await screenshot('new-local-day-applied');
        await undoApplied();
        const undone = await capture('new-day-undone');
        check(undone.events.length === 0 && undone.tasks.every(task => task.planning.time_block_event_ids.length === 0), 'new-day plan and Undo retain valid task/event relationships', [], undone);
      } else if (spec.kind.startsWith('stale-')) {
        state.phase = 'newer durable state while review stays open';
        const original = before.tasks.find(item => item.id === mainId);
        await Promise.all([page.evaluate(value => { globalThis.__todayQaNow = value; }, now + 60_000), writer.evaluate(value => { globalThis.__todayQaNow = value; }, now + 60_000)]);
        if (spec.kind === 'stale-calendar-conflict') await writeDB(writer, { puts: { events: [event(`${spec.id}-new-conflict`, 'New independent 09:00 calendar commitment', wall.start, wall.start + 45 * 60_000, now + 60_000)] } });
        if (spec.kind === 'stale-task-deleted') await writeDB(writer, { deletes: { work_items: [mainId] } });
        if (spec.kind === 'stale-task-completed') await writeDB(writer, { puts: { work_items: [{ ...original, status: 'Done', updated_at: now + 60_000 }] } });
        const newer = await capture('newer-committed-state-before-stale-apply');
        check(await review.isVisible(), 'primary page retains the stale proposal while the second page commits newer durable data', true, await review.isVisible());
        state.phase = 'stale Apply';
        const outcome = await apply(), after = await capture('after-stale-apply');
        check(equal(after.tasks, newer.tasks), 'stale Apply preserves newer task records, including deletion/completion', newer.tasks, after.tasks);
        check(equal(after.events, newer.events), 'stale Apply creates no overlapping, orphan, or extra calendar block', newer.events, after.events);
        const alerts = [...await today.getByRole('alert').allTextContents(), ...await review.getByRole('alert').allTextContents()];
        const appliedVisible = await today.getByText('Day plan applied.', { exact: true }).isVisible();
        check(!appliedVisible && !await undo.isVisible(), 'stale proposal is refused rather than reported as successfully applied', { appliedBanner: false, undoOffered: false }, { appliedBanner: appliedVisible, undoOffered: await undo.isVisible(), outcome });
        check(alerts.some(text => /refresh|review|changed|conflict|delet|complet|no longer|stale|missing|ready/i.test(text)), 'stale Apply explains why a refreshed review is required', 'actionable stale/conflict/task-change feedback', alerts);
        await screenshot('stale-apply-result');
        await page.reload();
        await today.getByRole('heading', { name: 'Today', exact: true }).waitFor();
        const reloaded = await capture('after-stale-apply-reload');
        check(equal(reloaded.tasks, newer.tasks) && equal(reloaded.events, newer.events), 'reload preserves the newer records without stale-plan writes', { tasks: newer.tasks, events: newer.events }, { tasks: reloaded.tasks, events: reloaded.events });
      } else {
        state.phase = 'apply commit';
        await apply();
        await undo.waitFor();
        const applied = await capture('applied-durable-state');
        const generated = applied.events.filter(item => !before.events.some(previous => previous.id === item.id));
        check(generated.length === tasks.filter(item => item.status !== 'Done').length, 'Apply durably creates exactly one block for each reviewed ready task', tasks.filter(item => item.status !== 'Done').length, generated);
        check(applied.tasks.filter(item => item.status !== 'Done').every(item => item.planning.time_block_event_ids.length === 1 && generated.some(block => block.id === item.planning.time_block_event_ids[0])), 'task links and calendar blocks commit together', 'one valid durable calendar link per proposed task', applied);
        await screenshot('applied');
        if (spec.kind === 'workday-boundaries') {
          state.phase = 'local workday boundaries';
          const actual = generated.map(item => ({ id: item.id, start_at_ms: item.start_at_ms, end_at_ms: item.end_at_ms, duration_minutes: (item.end_at_ms - item.start_at_ms) / 60_000 })).sort((a, b) => a.start_at_ms - b.start_at_ms);
          const local = await page.evaluate(events => events.map(item => ({ ...item, startLocal: new Date(item.start_at_ms).toLocaleString('en-US', { timeZoneName: 'short' }), endLocal: new Date(item.end_at_ms).toLocaleString('en-US', { timeZoneName: 'short' }) })), actual);
          check(actual.length === 6 && actual[0]?.start_at_ms === wall.start, 'WASM planner starts a pre-workday full-day proposal at 09:00 local wall time', { expectedStartMs: wall.start, localStart: `${spec.date} 09:00`, timezoneId: spec.timezoneId }, local);
          check(actual.length === 6 && actual.at(-1)?.end_at_ms === wall.end, 'WASM planner ends the six-block workday at 18:00 local wall time', { expectedEndMs: wall.end, localEnd: `${spec.date} 18:00`, timezoneId: spec.timezoneId }, local);
          check(actual.every(item => item.duration_minutes === 90 && item.start_at_ms >= wall.start && item.end_at_ms <= wall.end), 'every DST/control block has a valid duration inside the local workday', { durationMinutes: 90, startAtOrAfter: wall.start, endAtOrBefore: wall.end }, local);
          await page.reload();
          await today.getByRole('heading', { name: 'Today', exact: true }).waitFor();
          const reloaded = await capture('workday-reloaded');
          check(equal(reloaded.events, applied.events) && equal(reloaded.tasks, applied.tasks), 'actual planned instants and links survive reload unchanged', { tasks: applied.tasks, events: applied.events }, { tasks: reloaded.tasks, events: reloaded.events });
        } else {
          let newer = applied;
          if (spec.kind.startsWith('undo-newer-')) {
            state.phase = 'newer edit after Apply';
            const later = now + 60_000;
            await Promise.all([page.evaluate(value => { globalThis.__todayQaNow = value; }, later), writer.evaluate(value => { globalThis.__todayQaNow = value; }, later)]);
            const edited = structuredClone(applied.tasks.find(item => item.id === mainId));
            edited.updated_at = later;
            if (spec.kind === 'undo-newer-task-edit') Object.assign(edited, { title: 'Owner changed title after planning — 林', description: 'Owner changed description after planning', status: 'Done', priority: 'Urgent', due_date: '2026-11-14' });
            if (spec.kind === 'undo-newer-planning-edit') {
              const manual = event(`${spec.id}-owner-block`, 'Owner-created block after planning', wall.start + 4 * 3_600_000, wall.start + 4.5 * 3_600_000, later);
              Object.assign(edited.planning, { cycle: 'Owner revision ✓', estimate_minutes: 120, waiting_on: 'Waiting for Zoë', dependency_ids: [doneId], time_block_event_ids: [...edited.planning.time_block_event_ids, manual.id] });
              await writeDB(writer, { puts: { work_items: [edited], events: [manual] } });
            } else await writeDB(writer, { puts: { work_items: [edited] } });
            newer = await capture('newer-edit-committed-before-undo');
          }
          state.phase = 'Undo';
          await controlGeometry(undo, 'Day-plan Undo');
          await undoApplied();
          const after = await capture('after-undo');
          const generatedIds = generated.map(item => item.id);
          const expectedTasks = structuredClone(newer.tasks).map(item => {
            if (generatedIds.some(id => item.planning.time_block_event_ids.includes(id))) item.planning.time_block_event_ids = item.planning.time_block_event_ids.filter(id => !generatedIds.includes(id));
            return item;
          });
          const expectedEvents = newer.events.filter(item => !generatedIds.includes(item.id));
          const minimumUpdates = Object.fromEntries((spec.kind === 'happy-apply-undo' ? before.tasks : newer.tasks).map(item => [item.id, item.updated_at]));
          check(equal(taskContent(after.tasks), taskContent(expectedTasks)) && after.tasks.every(item => item.updated_at >= minimumUpdates[item.id]), 'Undo preserves newer task/planning edits and reverses only plan-owned task links', { records: expectedTasks, timestampPolicy: 'updated_at may advance; it cannot regress below a newer independent edit', minimumUpdates }, after.tasks);
          check(equal(after.events, expectedEvents), 'Undo removes unchanged generated blocks and preserves independent newer calendar records', expectedEvents, after.events);
          await screenshot('undo-result');
          await page.reload();
          await today.getByRole('heading', { name: 'Today', exact: true }).waitFor();
          const reloaded = await capture('undo-reloaded');
          check(equal(taskContent(reloaded.tasks), taskContent(expectedTasks)) && equal(reloaded.tasks, after.tasks) && equal(reloaded.events, expectedEvents), 'Undo result and preserved newer edits survive a full reload', { taskContent: taskContent(expectedTasks), exactPersistedTasks: after.tasks, events: expectedEvents }, { tasks: reloaded.tasks, events: reloaded.events });
        }
      }
    } catch (error) {
      check(false, 'scenario completes without an unexpected harness/UI interaction failure', 'successful setup and interaction', { error: error.stack ?? String(error) });
      await screenshot('failure').catch(() => {});
    } finally {
      state.phase = 'runtime and isolation';
      check(state.runtimeErrors.length === 0, 'no JavaScript/WASM runtime errors', [], state.runtimeErrors);
      check(state.consoleErrors.length === 0, 'no unexpected browser console errors', [], state.consoleErrors);
      check(state.blockedWrites.length === 0, 'Today planning attempts no external writes', [], state.blockedWrites);
      check(state.unexpected.length === 0, 'no unexpected external content/provider request', [], state.unexpected);
      await context.close();
      await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
      console.log(JSON.stringify({ case: spec.id, passed: report.checks.filter(check => check.case === spec.id && check.passed).length, failed: report.checks.filter(check => check.case === spec.id && !check.passed).length }));
    }
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
report.finishedAt = new Date().toISOString();
report.summary = { passed: report.checks.filter(check => check.passed).length, failed: report.checks.filter(check => !check.passed).length, cases: report.cases.length, screenshots: report.cases.reduce((count, item) => count + item.screenshots.length, 0) };
await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
const artifacts = await Promise.all((await readdir(output)).filter(name => name !== 'artifact-sha256.json').sort().map(async name => ({ name, sha256: digest(await readFile(path.join(output, name))) })));
await writeFile(path.join(output, 'artifact-sha256.json'), JSON.stringify({ recordedAt: new Date().toISOString(), harnessSha256: report.harnessSha256, artifacts }, null, 2) + '\n');
console.log(JSON.stringify({ ...report.summary, artifacts: output, scope: report.scope }));
if (report.summary.failed) process.exitCode = 1;
