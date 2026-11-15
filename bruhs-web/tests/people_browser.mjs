// People page acceptance on the real WASM bundle and IndexedDB cache.
// Synthetic contacts only: every /api read and the People mutations (create,
// tracked, rename, aliases, merge, unmerge, delete, restore, resolve-merges)
// are answered by in-process fixtures; any other write or non-loopback
// request fails the run.
// Run after: (cd bruhs-web && trunk build --dist ../target/people-ui-dist)
//   node bruhs-web/tests/people_browser.mjs
// BRUHS_PEOPLE_UI_DIST picks the bundle and BRUHS_PEOPLE_QA_DIR the output
// directory, so a later run never overwrites an earlier run's screenshots
// and results.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_PEOPLE_UI_DIST ?? path.join(root, 'target/people-ui-dist'));
const output = path.resolve(process.env.BRUHS_PEOPLE_QA_DIR ?? path.join(root, 'target/people-ui-qa'));
const startedAt = new Date().toISOString();
const cells = [
  { name: 'desktop', width: 1280, height: 800, hasTouch: false },
  { name: 'compact', width: 640, height: 480, hasTouch: false },
  { name: 'touch', width: 390, height: 844, hasTouch: true },
];
const relevantSources = [
  'bruhs-web/src/components/people_page.rs',
  'bruhs-web/src/components/people_page/directory.rs',
  'bruhs-web/src/components/people_page/merge_status.rs',
  'bruhs-web/src/components/people_page/back.rs',
  'bruhs-web/src/components/people_page/confirm.rs',
  'bruhs-web/src/components/people_page/profile.rs',
  'bruhs-web/src/components/people_page/relationship.rs',
  'bruhs-web/src/storage/people.rs',
  'bruhs-web/src/storage/people/changes.rs',
  'bruhs-web/src/storage/people/merge/commit.rs',
  'bruhs-web/src/storage/people/merge/undo.rs',
  'bruhs-web/src/workflow_executor/people_actions.rs',
  'bruhs-models/src/models/people_directory.rs',
  'bruhs-models/src/models/people_changes.rs',
  'bruhs-web/tailwind.css',
  'bruhs-models/src/models/person.rs',
];
await mkdir(output, { recursive: true });
const sourceTimes = await Promise.all(relevantSources.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs, sha256: createHash('sha256').update(await readFile(path.join(root, file))).digest('hex') })));
const indexStat = await stat(path.join(dist, 'index.html'));
assert(indexStat.mtimeMs >= Math.max(...sourceTimes.map(source => source.modifiedMs)), 'Refuse stale People WASM assets: rebuild target/people-ui-dist first');
const assetNames = (await readdir(dist)).filter(name => /\.(wasm|js|css|html)$/.test(name));
const assets = await Promise.all(assetNames.map(async name => ({ name, sha256: createHash('sha256').update(await readFile(path.join(dist, name))).digest('hex') })));
assert(assets.some(asset => asset.name.endsWith('.wasm')), 'People dist must contain a compiled WASM asset');

let playwright;
try { playwright = await import('playwright'); } catch {
  for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
    try { playwright = await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); break; } catch {}
  }
}
assert(playwright, 'An existing Playwright installation is required');

const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*' });
    response.write(': fixture\n\n');
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
const results = { startedAt, origin, dist, assets, sourceTimes, cells: [], checks: [] };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]'].includes(hostname);

// Synthetic fixtures. Two Decision observations share one meeting and one
// timestamp, as every observation from one summary does.
function fixtures() {
  const meeting = { id: 'source-meeting', title: 'Synthetic planning sync', started_at_ms: 1_700_000_000_000, ended_at_ms: 1_700_000_600_000, status: 'ended', calendar_event_id: null, audio_path: null, language: null, speakers: [], summary: null, thread_id: null, skip_observations: false, created_at: 1_700_000_000_000, updated_at: 1_700_000_600_000 };
  const observation = (at_meeting_id, kind, text) => ({ at_meeting_id, kind, text, created_at: 1_700_000_600_000 });
  const person = (id, display_name, tracked, observations, updated_at = 1_700_000_600_000, aliases = []) => ({ id, display_name, aliases, voice_embedding: null, observations, tracked, created_at: 1_700_000_000_000, updated_at });
  // Thirty older directory entries push the alias match onto page two.
  const directory = Array.from({ length: 30 }, (_, n) => {
    const label = String(n).padStart(2, '0');
    return [`dir-${label}`, person(`dir-${label}`, `Directory Person ${label}`, true, [], 1_600_000_000_000 - n)];
  });
  return {
    meetings: [meeting],
    people: new Map([
      ['ada-synthetic', person('ada-synthetic', 'Ada Synthetic', true, [observation('source-meeting', 'topic', 'Synthetic topic about the launch plan')])],
      ['opted-out', person('opted-out', 'Opted Out', false, [
        observation('source-meeting', 'decision', 'Synthetic decision one'),
        observation('source-meeting', 'decision', 'Synthetic decision two'),
        observation('deleted-meeting', 'tone', 'Synthetic tone from a deleted meeting'),
      ])],
      ...directory,
      ['needle', person('needle', 'Quiet Synthetic', true, [], 1_500_000_000_000, ['Lighthouse Keeper'])],
    ]),
    failTrackedOnce: true,
    trackedRequests: [],
    renameRequests: [],
    createRequests: [],
    aliasRequests: [],
    mergeRequests: [],
    unmergeRequests: [],
    deleteRequests: [],
    restoreRequests: [],
    merged: new Map(),
    deleted: new Map(),
  };
}

try {
  for (const cell of cells) {
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: cell.hasTouch, serviceWorkers: 'block', locale: 'en-US', timezoneId: 'America/Los_Angeles' });
    const state = { ...cell, stage: 'startup', screenshots: [], runtimeErrors: [], consoleErrors: [], unexpectedRequests: [], mockedReads: [], blockedWrites: [], layout: [] };
    const data = fixtures();
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
      const json = (value, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS, POST, PATCH, PUT, DELETE', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
      const api = url.pathname.startsWith('/api/') && loopback(url.hostname);
      if (api && method === 'OPTIONS') return json({});
      if (api && method === 'POST' && url.pathname === '/api/people/profiles') {
        const ids = JSON.parse(request.postData() ?? '{}').ids ?? [];
        state.mockedReads.push({ method, path: url.pathname, ids });
        return json(ids.filter(id => data.people.has(id)).map(id => data.people.get(id)));
      }
      if (api && method === 'POST' && url.pathname === '/api/people/resolve-merges') { state.mockedReads.push({ method, path: url.pathname }); return json([]); }
      const tracked = url.pathname.match(/^\/api\/people\/([^/]+)\/tracked$/);
      if (api && method === 'PATCH' && tracked) {
        const id = decodeURIComponent(tracked[1]);
        const body = JSON.parse(request.postData() ?? '{}');
        data.trackedRequests.push({ id, body });
        if (data.failTrackedOnce) { data.failTrackedOnce = false; return json({ error: 'Synthetic storage failure' }, 500); }
        const person = { ...data.people.get(id), tracked: body.tracked, updated_at: data.people.get(id).updated_at + 1 };
        data.people.set(id, person);
        return json(person);
      }
      const rename = url.pathname.match(/^\/api\/people\/([^/]+)\/rename$/);
      if (api && method === 'PATCH' && rename) {
        const id = decodeURIComponent(rename[1]);
        const body = JSON.parse(request.postData() ?? '{}');
        data.renameRequests.push({ id, body });
        const person = { ...data.people.get(id), display_name: body.display_name, updated_at: data.people.get(id).updated_at + 1 };
        data.people.set(id, person);
        return json(person);
      }
      const nameKey = name => name.trim().toLowerCase();
      if (api && method === 'POST' && url.pathname === '/api/people') {
        const body = JSON.parse(request.postData() ?? '{}');
        data.createRequests.push(body);
        if (data.people.has(body.id)) return json(data.people.get(body.id));
        const created = { id: body.id, display_name: body.display_name.trim(), aliases: [], voice_embedding: null, observations: [], tracked: true, created_at: body.created_at, updated_at: body.updated_at };
        data.people.set(body.id, created);
        return json(created, 201);
      }
      const aliases = url.pathname.match(/^\/api\/people\/([^/]+)\/aliases$/);
      if (api && method === 'PUT' && aliases) {
        const id = decodeURIComponent(aliases[1]);
        const body = JSON.parse(request.postData() ?? '{}');
        data.aliasRequests.push({ id, aliases: body.aliases });
        const clash = [...data.people.values()].find(other => other.id !== id && body.aliases.some(alias => [other.display_name, ...other.aliases].some(name => nameKey(name) === nameKey(alias))));
        if (clash) return json({ error: `${clash.display_name} already uses that name. Merge the two profiles or choose another alias. Nothing was added.` }, 409);
        const updated = { ...data.people.get(id), aliases: body.aliases, updated_at: data.people.get(id).updated_at + 1 };
        data.people.set(id, updated);
        return json(updated);
      }
      const mergeInto = url.pathname.match(/^\/api\/people\/([^/]+)\/merge_into\/([^/]+)$/);
      if (api && method === 'POST' && mergeInto) {
        const [sourceId, targetId] = [decodeURIComponent(mergeInto[1]), decodeURIComponent(mergeInto[2])];
        data.mergeRequests.push({ sourceId, targetId });
        const source = data.people.get(sourceId), target = data.people.get(targetId);
        const merged = { ...target, aliases: [...target.aliases, source.display_name], observations: [...target.observations, ...source.observations], tracked: target.tracked && source.tracked, updated_at: Math.max(target.updated_at, source.updated_at) + 1 };
        data.merged.set(sourceId, { source, target });
        data.people.delete(sourceId);
        data.people.set(targetId, merged);
        return json(merged);
      }
      const unmerge = url.pathname.match(/^\/api\/people\/([^/]+)\/unmerge$/);
      if (api && method === 'POST' && unmerge) {
        const sourceId = decodeURIComponent(unmerge[1]);
        data.unmergeRequests.push(sourceId);
        const original = data.merged.get(sourceId);
        if (!original) return json({ error: 'No merge to undo.' }, 404);
        const version = data.people.get(original.target.id).updated_at + 1;
        const source = { ...original.source, updated_at: version }, target = { ...original.target, updated_at: version };
        data.merged.delete(sourceId);
        data.people.set(source.id, source);
        data.people.set(target.id, target);
        return json({ source, target });
      }
      const restore = url.pathname.match(/^\/api\/people\/([^/]+)\/restore$/);
      if (api && method === 'POST' && restore) {
        const id = decodeURIComponent(restore[1]);
        data.restoreRequests.push(id);
        const deleted = data.deleted.get(id);
        if (!deleted) return json({ error: 'Nothing to restore.' }, 404);
        const restored = { ...deleted, updated_at: deleted.updated_at + 1 };
        data.deleted.delete(id);
        data.people.set(id, restored);
        return json(restored);
      }
      const single = url.pathname.match(/^\/api\/people\/([^/]+)$/);
      if (api && method === 'DELETE' && single) {
        const id = decodeURIComponent(single[1]);
        data.deleteRequests.push(id);
        const deleted = data.people.get(id);
        if (!deleted) return json({ error: 'person not found' }, 404);
        data.people.delete(id);
        data.deleted.set(id, deleted);
        return json({ person: deleted, speakers: [], action_items: [], deleted_at: Date.now() });
      }
      if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
        state.blockedWrites.push({ method, url: request.url(), body: request.postData() });
        return json({ error: 'People browser verification rejects unexpected writes.' }, 500);
      }
      if (api) {
        state.mockedReads.push({ method, path: url.pathname });
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
        if (url.pathname === '/api/people/versions') return json({ entries: [...data.people.values()].map(({ id, updated_at }) => ({ id, updated_at })).sort((a, b) => a.id < b.id ? -1 : 1), next_after: null });
        if (url.pathname === '/api/people') return json([...data.people.values()]);
        if (url.pathname === '/api/meetings') return json(data.meetings);
        if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        if (url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        if (/\/(credentials|tasks|threads|skills|models)$/.test(url.pathname)) return json([]);
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
    const card = id => page.locator('li').filter({ has: page.locator(`#person-record-${id}`) });
    const button = name => page.getByRole('button', { name, exact: true });
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
      await page.locator('#person-record-opted-out').waitFor();
      await page.locator('#person-record-ada-synthetic').waitFor();
      await dismissSidebar();
    }
    async function eventually(predicate, label, evidence = async () => undefined) {
      const deadline = Date.now() + 6000;
      do {
        if (await predicate()) return check(true, label);
        await sleep(80);
      } while (Date.now() < deadline);
      return check(false, label, await evidence());
    }
    async function reachable(locator, label) {
      await locator.scrollIntoViewIfNeeded();
      const evidence = await locator.evaluate(element => {
        const rect = element.getBoundingClientRect();
        let left = Math.max(0, rect.left), right = Math.min(innerWidth, rect.right), top = Math.max(0, rect.top), bottom = Math.min(innerHeight, rect.bottom);
        for (let parent = element.parentElement; parent; parent = parent.parentElement) {
          const style = getComputedStyle(parent), box = parent.getBoundingClientRect();
          if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) { left = Math.max(left, box.left); right = Math.min(right, box.right); }
          if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) { top = Math.max(top, box.top); bottom = Math.min(bottom, box.bottom); }
          if (style.position === 'fixed') break;
        }
        const hit = document.elementFromPoint((left + right) / 2, (top + bottom) / 2);
        return { rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, visibleWidth: right - left, visibleHeight: bottom - top, hit: Boolean(hit && (hit === element || element.contains(hit))) };
      });
      check(evidence.visibleWidth > 0 && evidence.visibleHeight > 0 && evidence.hit, `${label} is reachable without clipping or an overlay`, evidence);
      if (cell.hasTouch) check(evidence.rect.width >= 43.9 && evidence.rect.height >= 43.9, `${label} has a 44px touch target`, evidence.rect);
    }
    async function controlsInsideCards(label) {
      const escaped = await page.evaluate(() => [...document.querySelectorAll('li:has([id^="person-record-"])')].flatMap(card => {
        const box = card.getBoundingClientRect();
        return [...card.querySelectorAll('button, [role="alert"], span')].map(element => ({ text: element.textContent.trim().slice(0, 40), rect: element.getBoundingClientRect() }))
          .filter(({ rect }) => rect.width > 0 && (rect.right > box.right + 0.5 || rect.left < box.left - 0.5))
          .map(({ text, rect }) => ({ text, left: rect.left, right: rect.right, cardLeft: box.left, cardRight: box.right }));
      }));
      check(escaped.length === 0, `${label}: card text and controls stay inside their card`, escaped);
    }
    async function noHorizontalOverflow(label) {
      await controlsInsideCards(label);
      const evidence = await page.evaluate(() => ({ scrollWidth: document.scrollingElement.scrollWidth, innerWidth }));
      state.layout.push({ label, ...evidence });
      check(evidence.scrollWidth <= evidence.innerWidth, `${label} has no horizontal page overflow`, evidence);
    }
    try {
      state.stage = 'directory';
      await page.goto(`${origin}/#/people`);
      await ready();
      check(await card('opted-out').getByText('Not tracked', { exact: true }).isVisible(), 'opted-out profile shows its Not tracked state in words');
      check(await card('ada-synthetic').getByText('Not tracked', { exact: true }).count() === 0, 'tracked profile shows no opt-out badge');
      for (const name of ['Rename Opted Out', 'Track Opted Out', 'Merge Opted Out into another profile', 'Enroll voice for Opted Out', 'Untrack Ada Synthetic']) {
        check(await button(name).count() === 1, `card action has the accessible name "${name}"`);
      }
      await reachable(button('Track Opted Out'), 'Track action');
      await reachable(button('Merge Opted Out into another profile'), 'Merge action');
      await noHorizontalOverflow('People directory');
      await snapshot('directory');

      state.stage = 'opt-out detail';
      await page.locator('#person-record-opted-out').click();
      const optOut = card('opted-out');
      await optOut.locator('[data-people-opt-out]').waitFor();
      check((await optOut.locator('[data-people-opt-out]').innerText()).includes('agents cannot read the saved ones'), 'expanded opt-out explains collection and agent access');
      await eventually(async () => await optOut.locator('[data-person-observation]').count() === 3, 'all three observations render, including two that share a meeting and timestamp', async () => ({ rendered: await optOut.locator('[data-person-observation]').allInnerTexts() }));
      for (const text of ['Synthetic decision one', 'Synthetic decision two', 'Synthetic tone from a deleted meeting']) {
        check(await optOut.getByText(text, { exact: true }).count() === 1, `observation "${text}" renders exactly once`);
      }
      const source = optOut.getByRole('button', { name: 'Open source meeting Synthetic planning sync', exact: true });
      check(await source.count() === 2, 'observations from a stored meeting link to it', { count: await source.count() });
      check(await optOut.getByText('Source meeting is not stored', { exact: true }).count() === 1, 'an observation from a deleted meeting says so instead of linking');
      check(state.mockedReads.some(read => read.path === '/api/meetings'), 'People loads meetings itself for provenance on a cold visit');
      await reachable(source.first(), 'Source meeting link');
      await noHorizontalOverflow('Expanded opt-out profile');
      await snapshot('opt-out-detail');

      state.stage = 'tracking failure';
      await button('Untrack Ada Synthetic').click();
      const alert = page.getByRole('alert', { name: 'Profile change failed', exact: true });
      await eventually(async () => (await alert.count()) === 1 && (await alert.innerText()).includes('Observation tracking was not changed'), 'a failed opt-out shows a visible error');
      check(await card('ada-synthetic').getByText('Not tracked', { exact: true }).count() === 0, 'a failed opt-out does not look applied');
      await snapshot('tracking-failure');

      state.stage = 'tracking success';
      await button('Untrack Ada Synthetic').click();
      await eventually(async () => await card('ada-synthetic').getByText('Not tracked', { exact: true }).isVisible(), 'a confirmed opt-out shows the Not tracked state');
      check(await alert.count() === 0, 'retrying clears the failure message');
      check(data.trackedRequests.length === 2 && data.trackedRequests.every(r => r.id === 'ada-synthetic' && r.body.tracked === false), 'both attempts sent tracked=false for the chosen profile', data.trackedRequests);
      check(await button('Track Ada Synthetic').count() === 1, 'the toggle now offers to track again');

      state.stage = 'rename validation';
      await button('Rename Ada Synthetic').click();
      const nameInput = page.getByRole('textbox', { name: 'Profile name', exact: true });
      await nameInput.fill('   ');
      await nameInput.press('Enter');
      await eventually(async () => (await alert.count()) === 1 && (await alert.innerText()).includes('A name is required'), 'a blank rename is rejected with a visible message');
      check(data.renameRequests.length === 0, 'a blank rename sends no request', data.renameRequests);
      await button('Rename Ada Synthetic').click();
      await nameInput.fill('  Ada Renamed  ');
      await nameInput.press('Enter');
      await eventually(async () => await page.locator('#person-record-ada-synthetic').getByText('Ada Renamed', { exact: true }).isVisible(), 'a valid rename shows the trimmed name');
      check(data.renameRequests.length === 1 && data.renameRequests[0].body.display_name === 'Ada Renamed', 'rename sends the trimmed name once', data.renameRequests);

      state.stage = 'reload';
      await page.reload();
      await ready();
      check(await page.locator('#person-record-ada-synthetic').getByText('Ada Renamed', { exact: true }).isVisible(), 'rename survives reload');
      check(await card('ada-synthetic').getByText('Not tracked', { exact: true }).isVisible(), 'opt-out survives reload');
      await snapshot('after-reload');

      state.stage = 'source navigation';
      await page.locator('#person-record-opted-out').click();
      await card('opted-out').getByRole('button', { name: 'Open source meeting Synthetic planning sync', exact: true }).first().click();
      await eventually(async () => (await page.evaluate(() => location.hash)) === '#/meetings/source-meeting', 'the source link opens the meeting detail', async () => ({ hash: await page.evaluate(() => location.hash) }));
      await snapshot('source-meeting');

      state.stage = 'create';
      await page.goto(`${origin}/#/people`);
      await ready();
      const createInput = page.getByRole('textbox', { name: 'New person', exact: true });
      await reachable(button('Add person'), 'Add person action');
      await createInput.fill('   ');
      await button('Add person').click();
      await eventually(async () => await page.getByText('A name is required. No profile was created.', { exact: true }).isVisible(), 'a blank create is refused with a visible message');
      check(await createInput.inputValue() === '   ', 'a refused create keeps the typed input');
      check(data.createRequests.length === 0, 'a blank create sends nothing', data.createRequests);
      const longName = 'Zoë Ångström Łukasiewicz Ōtsuka, a deliberately long synthetic name that keeps going across the card';
      await createInput.fill(longName);
      await button('Add person').click();
      const createdCard = () => page.locator('[id^="person-record-"]').filter({ hasText: longName });
      await eventually(async () => await createdCard().count() === 1, 'the created profile is listed and opened');
      await eventually(async () => data.createRequests.some(request => request.display_name === longName), 'the created profile syncs with its Unicode name', async () => data.createRequests);
      check(await createInput.inputValue() === '', 'a successful create clears the input');
      await noHorizontalOverflow('Long Unicode profile name');
      await snapshot('created');
      await page.reload();
      await ready();
      await eventually(async () => await createdCard().count() === 1, 'the created profile survives reload');

      state.stage = 'search';
      const search = page.getByRole('searchbox', { name: 'Search people', exact: true });
      check(await page.locator('#person-record-needle').count() === 0, 'the alias match starts beyond the first page');
      check(await page.getByRole('navigation', { name: 'People pages', exact: true }).getByText('Page 1 of 2', { exact: true }).isVisible(), 'the directory pages at 25 people');
      await search.fill('LIGHTHOUSE');
      await eventually(async () => await page.locator('#person-record-needle').count() === 1 && await page.locator('[id^="person-record-"]').count() === 1, 'search by alias finds a person beyond the first page, ignoring case');
      await search.fill('zoË ÅNGSTRÖM');
      await eventually(async () => await createdCard().count() === 1 && await page.locator('[id^="person-record-"]').count() === 1, 'search folds Unicode case');
      await search.fill('nobody by this name');
      await eventually(async () => await page.getByText('No people match this search.', { exact: true }).isVisible(), 'a search with no match says so');
      await search.fill('');
      await eventually(async () => await page.locator('#person-record-ada-synthetic').count() === 1, 'clearing the search restores the directory');
      await snapshot('search');

      state.stage = 'aliases';
      await page.locator('#person-record-ada-synthetic').click();
      const ada = card('ada-synthetic');
      const aliasInput = ada.getByRole('textbox', { name: 'New alias', exact: true });
      await aliasInput.fill('opted OUT');
      await ada.getByRole('button', { name: 'Add alias', exact: true }).click();
      const aliasAlert = ada.getByRole('alert', { name: 'Alias not added', exact: true });
      await eventually(async () => (await aliasAlert.count()) === 1 && (await aliasAlert.innerText()).includes('Opted Out already uses that name'), 'an alias another profile uses is refused with a visible message');
      check(data.aliasRequests.length === 0, 'a refused alias sends nothing', data.aliasRequests);
      check(await aliasInput.inputValue() === 'opted OUT', 'a refused alias stays in its input');
      await aliasInput.fill('Countess Ada');
      await ada.getByRole('button', { name: 'Add alias', exact: true }).click();
      await eventually(async () => await ada.getByRole('button', { name: 'Remove alias Countess Ada', exact: true }).count() === 1, 'a saved alias is listed with its own remove control');
      check(data.aliasRequests.at(-1)?.aliases.includes('Countess Ada'), 'the alias list is saved on the server', data.aliasRequests);
      await reachable(ada.getByRole('button', { name: 'Remove alias Countess Ada', exact: true }), 'Remove alias action');
      await search.fill('countess');
      await eventually(async () => await page.locator('#person-record-ada-synthetic').count() === 1 && await page.locator('[id^="person-record-"]').count() === 1, 'search finds the new alias');
      await search.fill('');
      await ada.getByRole('button', { name: 'Remove alias Countess Ada', exact: true }).click();
      await eventually(async () => await ada.getByText('No aliases yet.', { exact: true }).isVisible(), 'removing the alias empties the list');
      await snapshot('aliases');

      state.stage = 'linked records';
      check(await ada.getByText('No Inbox conversations with this person yet.', { exact: true }).isVisible(), 'a profile with no conversations says so');
      const meetingLink = ada.getByRole('button', { name: 'Open meeting Synthetic planning sync', exact: true });
      await reachable(meetingLink, 'Linked meeting');
      await meetingLink.click();
      await eventually(async () => (await page.evaluate(() => location.hash)) === '#/meetings/source-meeting', 'a linked meeting opens from the profile');
      await button('Back to People').click();
      await eventually(async () => (await page.evaluate(() => location.hash)) === '#/people' && (await page.locator('#person-record-ada-synthetic').getAttribute('aria-expanded')) === 'true', 'Back to People returns to the open profile');

      state.stage = 'reminders';
      await ada.getByRole('button', { name: 'Reminder', exact: true }).first().click();
      await ada.getByRole('checkbox', { name: 'Due tomorrow', exact: true }).check();
      await ada.getByRole('textbox', { name: 'New relationship item', exact: true }).fill('Send the synthetic brief');
      await ada.getByRole('button', { name: 'Add', exact: true }).click();
      await eventually(async () => await ada.getByRole('button', { name: 'Complete Send the synthetic brief', exact: true }).count() === 1, 'a reminder is added');
      check(await ada.getByText('Reminder, due', { exact: true }).isVisible(), 'the reminder shows its kind and due state');
      await ada.getByRole('button', { name: 'Edit Send the synthetic brief', exact: true }).click();
      const itemText = ada.getByRole('textbox', { name: 'Item text', exact: true });
      await itemText.fill('   ');
      await ada.getByRole('button', { name: 'Save', exact: true }).click();
      await eventually(async () => await ada.getByText('Text is required. The item was not changed.', { exact: true }).isVisible(), 'a blank reminder edit is refused');
      await itemText.fill('Send the revised brief');
      await ada.getByRole('button', { name: 'Commitment', exact: true }).last().click();
      await ada.getByRole('button', { name: 'Save', exact: true }).click();
      await eventually(async () => await ada.getByRole('button', { name: 'Complete Send the revised brief', exact: true }).count() === 1, 'an edited reminder keeps its row with new text');
      check(await ada.getByText('Commitment, due', { exact: true }).isVisible(), 'the edit changed the kind and kept the due time');
      await page.reload();
      await ready();
      await page.locator('#person-record-ada-synthetic').click();
      await eventually(async () => await card('ada-synthetic').getByRole('button', { name: 'Complete Send the revised brief', exact: true }).count() === 1, 'an edited reminder survives reload');
      await card('ada-synthetic').getByRole('button', { name: 'Complete Send the revised brief', exact: true }).click();
      await eventually(async () => await card('ada-synthetic').getByRole('button', { name: 'Reopen Send the revised brief', exact: true }).count() === 1, 'a reminder can be completed');
      await reachable(card('ada-synthetic').getByRole('button', { name: 'Delete Send the revised brief', exact: true }), 'Delete reminder action');
      await card('ada-synthetic').getByRole('button', { name: 'Delete Send the revised brief', exact: true }).click();
      await eventually(async () => await card('ada-synthetic').locator('[data-relationship-item]').count() === 0, 'a reminder can be deleted');
      await noHorizontalOverflow('Expanded profile with aliases, links and reminders');
      await snapshot('reminders');
      await page.locator('#person-record-ada-synthetic').click();

      state.stage = 'merge';
      await button('Merge Directory Person 00 into another profile').click();
      await card('dir-00').getByRole('button', { name: 'Merge into', exact: true }).click();
      await page.getByRole('option', { name: 'Opted Out', exact: true }).click();
      await card('dir-00').getByRole('button', { name: 'Merge', exact: true }).click();
      await eventually(async () => (await card('dir-00').getByText('Merge Directory Person 00 into Opted Out?', { exact: false }).count()) === 1, 'merge asks for confirmation in words');
      check(data.mergeRequests.length === 0, 'nothing merges before confirmation', data.mergeRequests);
      await reachable(button('Confirm merge'), 'Confirm merge action');
      await snapshot('merge-confirm');
      await button('Confirm merge').click();
      await eventually(async () => data.mergeRequests.length === 1 && await page.locator('#person-record-dir-00').count() === 0, 'a confirmed merge retires the source profile');
      await eventually(async () => await button('Undo merge').count() === 1, 'a merge offers undo');
      await button('Undo merge').click();
      // A restored profile is new to this session's directory order, so it
      // joins the last page; find it by name.
      await search.fill('Directory Person 00');
      await eventually(async () => data.unmergeRequests.length === 1 && await page.locator('#person-record-dir-00').count() === 1, 'undo brings the merged profile back');
      await search.fill('');
      check(await button('Undo merge').count() === 0, 'a finished undo clears its offer');

      state.stage = 'delete';
      await button('Delete Directory Person 01').click();
      await eventually(async () => (await card('dir-01').getByText('Delete Directory Person 01?', { exact: false }).count()) === 1, 'delete asks for confirmation with the cascade spelled out');
      check(data.deleteRequests.length === 0, 'nothing is deleted before confirmation', data.deleteRequests);
      await reachable(button('Delete profile'), 'Delete profile action');
      await snapshot('delete-confirm');
      await button('Delete profile').click();
      await eventually(async () => data.deleteRequests.length === 1 && await page.locator('#person-record-dir-01').count() === 0, 'a confirmed delete removes the profile');
      await eventually(async () => await button('Undo delete').count() === 1, 'a delete offers undo');
      await reachable(button('Undo delete'), 'Undo delete action');
      await button('Undo delete').click();
      await search.fill('Directory Person 01');
      await eventually(async () => data.restoreRequests.length === 1 && await page.locator('#person-record-dir-01').count() === 1, 'undo restores the deleted profile');
      await search.fill('');
      await page.reload();
      await ready();
      check(await page.locator('#person-record-dir-01').count() === 1 && await page.locator('#person-record-dir-00').count() === 1, 'restored profiles survive reload');
      await noHorizontalOverflow('Directory after merge and delete undo');
      await snapshot('after-undo');
    } catch (error) {
      check(false, 'run completed without an uncaught harness error', { error: error.stack ?? String(error) });
      await snapshot('failure').catch(() => {});
    } finally {
      state.stage = 'teardown';
      check(state.runtimeErrors.length === 0, 'no runtime errors', state.runtimeErrors);
      check(state.blockedWrites.length === 0, 'no unexpected writes', state.blockedWrites);
      check(state.unexpectedRequests.length === 0, 'no non-loopback requests', state.unexpectedRequests);
      await context.close();
      console.log(JSON.stringify({ cell: cell.name, passed: results.checks.filter(c => c.cell === cell.name && c.passed).length, failed: results.checks.filter(c => c.cell === cell.name && !c.passed).length, screenshots: state.screenshots }));
      await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2) + '\n');
    }
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
results.finishedAt = new Date().toISOString();
results.summary = { passed: results.checks.filter(c => c.passed).length, failed: results.checks.filter(c => !c.passed).length, cells: results.cells.length, screenshots: results.cells.reduce((count, c) => count + c.screenshots.length, 0) };
await writeFile(path.join(output, 'results.json'), JSON.stringify(results, null, 2) + '\n');
console.log(JSON.stringify({ ...results.summary, artifacts: output }));
if (results.summary.failed) process.exitCode = 1;
