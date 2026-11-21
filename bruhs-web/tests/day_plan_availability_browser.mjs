// Actual Today UI and IndexedDB against independently declared availability.
// Synthetic records and read-only API fixtures; no provider or external writes.
// BRUHS_PLAYWRIGHT_PATH=/existing/playwright node ...mjs /fresh/trunk/dist
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
assert(process.argv[2], 'Pass an explicitly built WASM dist');
const dist = path.resolve(process.argv[2]);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const output = path.join(root, 'target/day-plan-availability', new Date().toISOString().replace(/[:.]/g, '-'));
await mkdir(output, { recursive: true });
const files = ['bruhs-models/src/models/planning.rs', 'bruhs-models/src/models/planning/day_plan.rs', 'bruhs-models/src/models/planning/day_plan/availability.rs', 'bruhs-models/src/models/scheduling.rs', 'bruhs-models/src/calendar/zone.rs', 'bruhs-web/src/components/today_page.rs', 'bruhs-web/src/task_saves/planning.rs', 'bruhs-web/src/storage/day_plans.rs', 'bruhs-web/src/state/calendar.rs', 'bruhs-web/src/clock.rs'];
const sources = await Promise.all(files.map(async file => ({ file, sha256: hash(await readFile(path.join(root, file))), modifiedMs: (await stat(path.join(root, file))).mtimeMs })));
assert((await stat(path.join(dist, 'index.html'))).mtimeMs >= Math.max(...sources.map(source => source.modifiedMs)), 'Refuse a stale web bundle');
const assets = await Promise.all((await readdir(dist)).filter(file => /\.(html|wasm|css|js)$/.test(file)).map(async file => ({ file, sha256: hash(await readFile(path.join(dist, file))) })));
const report = { output, dist, sources, assets, harnessSha256: hash(await readFile(fileURLToPath(import.meta.url))), checks: [], cases: [], scope: 'Actual Today UI and isolated IndexedDB with synthetic meeting-sourced tasks, explicit working hours, calendar commitments, stale preference writes and Undo. API replies are read-only fixtures; no live provider, native application or competitor comparison.' };
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://fixture');
  if (url.pathname === '/writer') { response.writeHead(200, { 'content-type': 'text/html' }); return response.end('<!doctype html><title>Isolated fixture writer</title>'); }
  if (url.pathname === '/stream') { response.writeHead(200, { 'content-type': 'text/event-stream', 'access-control-allow-origin': '*' }); response.write(': fixture\n\n'); return; }
  const file = path.resolve(dist, `.${url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)}`);
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); return response.end(); }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'content-type': { '.html': 'text/html', '.js': 'application/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' }[path.extname(file)] || 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = await chromium.launch({ headless: true });
report.browser = browser.version();
const rule = (start, end, weekdays = [1, 2, 3, 4, 5]) => ({ id: `${start}-${end}`, label: 'Fixture working hours', weekdays, start_minute: start, end_minute: end, enabled: true });
const normal = [rule(540, 1020)];
const specifications = [
  { name: 'closing-time', rules: normal, minutes: 60, busy: ['2026-11-16T17:00:00Z', '2026-11-17T00:30:00Z'], expected: null },
  { name: 'split-hours', rules: [rule(540, 600), rule(840, 960)], minutes: 90, expected: ['2026-11-16T22:00:00Z', '2026-11-16T23:30:00Z'] },
  { name: 'weekend', rules: normal, minutes: 45, now: '2026-11-21T16:00:00Z', expected: null },
  { name: 'early-hours', rules: [rule(420, 480)], minutes: 45, now: '2026-11-16T14:00:00Z', expected: ['2026-11-16T15:00:00Z', '2026-11-16T15:45:00Z'] },
  { name: 'late-hours', rules: [rule(1140, 1260)], minutes: 60, now: '2026-11-17T02:30:00Z', expected: ['2026-11-17T03:00:00Z', '2026-11-17T04:00:00Z'] },
  { name: 'adjacent-hours', rules: [rule(540, 600), rule(600, 660)], minutes: 90, expected: ['2026-11-16T17:00:00Z', '2026-11-16T18:30:00Z'] },
  { name: 'changed-hours-before-apply', rules: normal, minutes: 60, changedRules: [rule(540, 570)], expected: null },
  { name: 'all-day-time-off', rules: normal, minutes: 45, busy: ['2026-11-16T08:00:00Z', '2026-11-17T08:00:00Z'], allDay: true, expected: null },
  { name: 'all-day-added-before-apply', rules: normal, minutes: 45, busy: ['2026-11-16T08:00:00Z', '2026-11-17T08:00:00Z'], allDay: true, addAfterReview: true, expected: null },
  { name: 'spring-forward-short-opening', rules: [rule(90, 210, [0])], minutes: 90, now: '2026-03-08T08:00:00Z', expected: null },
  { name: 'fall-back-interrupted-opening', rules: [rule(90, 120, [0])], minutes: 45, now: '2026-11-01T07:00:00Z', expected: null },
  { name: 'unreadable-hours-before-apply', rules: normal, minutes: 60, corruptPreferences: true, expected: null },
  { name: 'disabled-hours', rules: normal.map(rule => ({ ...rule, enabled: false })), minutes: 45, expected: null },
  { name: 'configured-weekend', rules: [rule(540, 1020, [6])], minutes: 45, now: '2026-11-21T16:00:00Z', expected: ['2026-11-21T17:00:00Z', '2026-11-21T17:45:00Z'] },
];
async function records(page, seed) {
  return page.evaluate(seed => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('The real app must create the schema')); };
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result, tx = db.transaction(['work_items', 'events'], seed ? 'readwrite' : 'readonly');
      if (seed) for (const [store, values] of Object.entries(seed)) for (const value of values) tx.objectStore(store).put(value);
      const tasks = tx.objectStore('work_items').getAll(), events = tx.objectStore('events').getAll();
      tx.oncomplete = () => { db.close(); resolve({ tasks: tasks.result, events: events.result }); };
      tx.onabort = () => { db.close(); reject(tx.error); };
    };
  }), seed);
}
const waitFor = async (test, description) => {
  const deadline = Date.now() + 12000;
  while (Date.now() < deadline) { if (await test()) return; await new Promise(done => setTimeout(done, 50)); }
  throw new Error(`Timed out: ${description}`);
};
try {
  for (const spec of specifications) {
    const current = { ...spec, errors: [], blockedWrites: [], checks: [] };
    report.cases.push(current);
    const check = (passed, label, actual) => { const value = { case: spec.name, label, passed: Boolean(passed), actual }; report.checks.push(value); current.checks.push(value); };
    const touch = process.env.BRUHS_DAY_PLAN_TOUCH === '1';
    const viewport = touch ? { width: 390, height: 844 } : { width: 1280, height: 900 };
    current.viewport = viewport;
    current.hasTouch = touch;
    const context = await browser.newContext({ viewport, hasTouch: touch, timezoneId: 'America/Los_Angeles', serviceWorkers: 'block' });
    const now = Date.parse(spec.now || '2026-11-16T16:00:00Z');
    const preferences = { id: 'default', availability: spec.rules, focus: { enabled: true, daily_target_minutes: 120, minimum_block_minutes: 45, preferred_start_minute: 540 }, links: [], conflicts: [], updated_at_ms: now };
    await context.addInitScript(({ now }) => {
      const NativeDate = Date;
      globalThis.Date = new Proxy(NativeDate, { construct: (target, args) => Reflect.construct(target, args.length ? args : [now]), apply: () => new NativeDate(now).toString(), get: (target, key) => key === 'now' ? () => now : Reflect.get(target, key) });
    }, { now });
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url());
      if (url.pathname.startsWith('/api/')) {
        if (!['GET', 'OPTIONS'].includes(request.method())) current.blockedWrites.push({ method: request.method(), path: url.pathname });
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/stream` });
        const value = url.pathname === '/api/reactive' ? { agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] } : url.pathname === '/api/weekly-reports' ? { presets: [], reports: [], next_before: null } : /\/(meetings|people|credentials|tasks|threads|skills|models|calendars)$/.test(url.pathname) ? [] : {};
        return route.fulfill({ status: ['GET', 'OPTIONS'].includes(request.method()) ? 200 : 403, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, OPTIONS' }, body: JSON.stringify(value) });
      }
      if (url.origin === origin) return route.continue();
      current.blockedWrites.push({ method: request.method(), url: request.url() });
      return route.abort();
    });
    const page = await context.newPage(), writer = await context.newPage();
    page.setDefaultTimeout(12000);
    page.on('pageerror', error => current.errors.push(String(error)));
    const dismissSidebar = async () => {
      const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
      if (await scrim.isVisible()) await scrim.click({ position: { x: viewport.width - 12, y: 60 } });
    };
    try {
      await page.goto(`${origin}/#/today`);
      await page.getByRole('heading', { name: 'Today', exact: true }).waitFor();
      await page.waitForFunction(async () => (await indexedDB.databases()).some(db => db.name === 'bruhs'));
      await writer.goto(`${origin}/writer`);
      await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', JSON.stringify(value)), preferences);
      const source = { kind: 'meeting', meeting_id: `meeting-${spec.name}`, title: 'Synthetic planning meeting' };
      const task = { id: `task-${spec.name}`, project_id: null, title: 'Implement the reviewed meeting action', description: 'Retain the meeting context', source, status: 'Todo', priority: 'High', due_date: null, planning: { cycle: null, dependency_ids: [], estimate_minutes: spec.minutes, recurrence: null, time_block_event_ids: [], waiting_on: null }, created_at: now, updated_at: now };
      const events = spec.busy ? [{ id: `busy-${spec.name}`, title: 'Existing commitment', start_at_ms: Date.parse(spec.busy[0]), end_at_ms: Date.parse(spec.busy[1]), all_day: Boolean(spec.allDay), color: null, notes: null, created_at: now, updated_at: now, source: 'local', google_event_id: null, calendar_id: null, attendees: [], organizer: null, location: null, conferencing: null, recurring_event_id: null, html_link: null }] : [];
      await records(writer, { work_items: [task], events: spec.addAfterReview ? [] : events });
      await page.reload();
      await page.locator('#focus-heading').locator('..').locator('..').getByText(task.title, { exact: true }).waitFor();
      await dismissSidebar();
      await page.getByRole('button', { name: 'Plan my day', exact: true }).click();
      const review = page.getByRole('dialog', { name: 'Review day plan', exact: true });
      await review.waitFor();
      current.review = await review.innerText();
      if (spec.addAfterReview) await records(writer, { events });
      if (spec.changedRules) await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', JSON.stringify(value)), { ...preferences, availability: spec.changedRules, updated_at_ms: now + 1 });
      if (spec.corruptPreferences) await writer.evaluate(() => localStorage.setItem('bruhs.scheduling', '{invalid-owner-preferences'));
      const beforeApply = await records(writer);
      current.beforeApply = beforeApply;
      const apply = review.getByRole('button', { name: 'Apply plan', exact: true });
      if (await apply.isEnabled()) {
        await apply.click();
        await waitFor(async () => (await page.getByRole('button', { name: 'Undo', exact: true }).count()) || (await page.getByRole('alert').count()), 'commit or rejection');
      }
      const saved = await records(writer), generated = saved.events.filter(event => !events.some(original => original.id === event.id));
      current.afterApply = saved;
      check(spec.expected ? generated.length === 1 && generated[0].start_at_ms === Date.parse(spec.expected[0]) && generated[0].end_at_ms === Date.parse(spec.expected[1]) : generated.length === 0, 'time blocks fit the independently declared working hours', { expected: spec.expected, generated });
      check(JSON.stringify(saved.tasks[0].source) === JSON.stringify(source), 'planning retains the task meeting identity', saved.tasks[0].source);
      check(events.every(original => saved.events.some(event => JSON.stringify(event) === JSON.stringify(original))), 'independent calendar commitments remain unchanged', saved.events);
      if (!spec.expected) check(JSON.stringify(saved) === JSON.stringify(beforeApply), 'an unavailable or rejected plan preserves every saved task and event field', saved);
      if (spec.changedRules || spec.corruptPreferences || spec.addAfterReview) {
        const alerts = await page.getByRole('alert').allTextContents();
        check(alerts.some(text => spec.changedRules ? /Working hours changed/.test(text) : spec.corruptPreferences ? /Working hours could not be read/.test(text) : /calendar changed/i.test(text)), 'changed availability or calendar causes actionable rejection', alerts);
        if (spec.corruptPreferences) check(await writer.evaluate(() => localStorage.getItem('bruhs.scheduling')) === '{invalid-owner-preferences', 'failed validation preserves the owner preference bytes', await writer.evaluate(() => localStorage.getItem('bruhs.scheduling')));
        if (spec.corruptPreferences) await writer.evaluate(value => localStorage.setItem('bruhs.scheduling', JSON.stringify(value)), { ...preferences, availability: [rule(540, 570)], updated_at_ms: now + 1 });
        if (await review.isVisible()) await review.getByRole('button', { name: 'Cancel', exact: true }).click();
        const refresh = page.getByRole('button', { name: 'Refresh saved tasks and calendar', exact: true });
        await refresh.click();
        await waitFor(async () => !await refresh.count() && await page.getByRole('button', { name: 'Plan my day', exact: true }).isEnabled(), 'refresh latest saved state');
        check(!await page.getByRole('status', { name: 'Notifications', exact: true }).getByText(alerts[0], { exact: true }).count(), 'successful refresh clears its stale error notification', await page.getByRole('status', { name: 'Notifications', exact: true }).innerText());
        await page.getByRole('button', { name: 'Plan my day', exact: true }).click();
        await review.waitFor();
        check(await review.getByText('No unscheduled ready work fits in the rest of today.', { exact: true }).count() && !await review.getByRole('button', { name: 'Apply plan', exact: true }).isEnabled(), 'a new review recovers using the latest saved availability and calendar', await review.innerText());
        const recovered = await records(writer);
        check(JSON.stringify(recovered) === JSON.stringify(saved), 'recovery does not rewrite the task, meeting source or calendar', recovered);
      }
      if (generated.length) {
        await page.getByRole('button', { name: 'Undo', exact: true }).click();
        await waitFor(async () => (await records(writer)).events.length === events.length, 'Undo to commit');
        await page.reload();
        await page.getByRole('heading', { name: 'Today', exact: true }).waitFor();
        await dismissSidebar();
        const restored = await records(writer);
        current.afterUndoReload = restored;
        check(restored.tasks[0].planning.time_block_event_ids.length === 0 && JSON.stringify(restored.tasks[0].source) === JSON.stringify(source) && restored.events.length === events.length, 'Undo and reload preserve source and remove only the plan links', restored);
      }
      const screenshot = `${spec.name}.png`;
      await page.screenshot({ path: path.join(output, screenshot) });
      current.screenshot = screenshot;
    } catch (error) {
      check(false, 'scenario completes', String(error.stack || error));
      current.screenshot = `${spec.name}-failure.png`;
      await page.screenshot({ path: path.join(output, current.screenshot) });
    }
    check(!current.errors.length && !current.blockedWrites.length, 'no runtime error or external mutation', { errors: current.errors, blockedWrites: current.blockedWrites });
    await context.close();
    await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify({ case: spec.name, passed: current.checks.filter(check => check.passed).length, failed: current.checks.filter(check => !check.passed).length }));
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(done => server.close(done));
}
report.summary = { passed: report.checks.filter(check => check.passed).length, failed: report.checks.filter(check => !check.passed).length };
await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify({ ...report.summary, output }));
if (report.summary.failed) process.exitCode = 1;
