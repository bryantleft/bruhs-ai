// Real production WASM + native IndexedDB. All API reads are local fixtures;
// every attempted remote mutation or unexpected network request fails the run.
// Run after: trunk build --release --dist ../target/inbox-ui-dist
//   node bruhs-web/tests/inbox_browser.mjs
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const dist = path.resolve(process.env.BRUHS_INBOX_UI_DIST ?? path.join(root, 'target/inbox-ui-dist'));
// BRUHS_INBOX_QA_DIR keeps a later run from overwriting earlier evidence.
const output = path.resolve(process.env.BRUHS_INBOX_QA_DIR ?? path.join(root, 'target/inbox-ui-qa'));
const startedAt = new Date().toISOString();
const cells = [
  { name: 'desktop', width: 1280, height: 800, hasTouch: false },
  { name: 'compact', width: 640, height: 480, hasTouch: false },
  { name: 'touch', width: 390, height: 844, hasTouch: true },
];
const relevantSources = [
  'bruhs-web/src/components/inbox_page.rs',
  'bruhs-web/src/components/task_pagination.rs',
  'bruhs-web/src/state/calendar.rs',
  'bruhs-web/src/state/workspace.rs',
  'bruhs-web/src/storage.rs',
  'bruhs-web/src/ui/dialog.rs',
  'bruhs-web/src/ui/button.rs',
  'bruhs-web/src/ui/input.rs',
  'bruhs-web/src/ui/textarea.rs',
  'bruhs-web/tailwind.css',
  'bruhs-models/src/models/inbox.rs',
  'bruhs-models/src/models/inbox/directory.rs',
];
await mkdir(output, { recursive: true });
const sourceTimes = await Promise.all(relevantSources.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs, sha256: createHash('sha256').update(await readFile(path.join(root, file))).digest('hex') })));
const indexStat = await stat(path.join(dist, 'index.html'));
assert(indexStat.mtimeMs >= Math.max(...sourceTimes.map(source => source.modifiedMs)), 'Refuse stale Inbox WASM assets: rebuild target/inbox-ui-dist first');
const assetNames = (await readdir(dist)).filter(name => /\.(wasm|js|css|html)$/.test(name));
const assets = await Promise.all(assetNames.map(async name => ({ name, sha256: createHash('sha256').update(await readFile(path.join(dist, name))).digest('hex') })));
assert(assets.some(asset => asset.name.endsWith('.wasm')), 'Inbox dist must contain a compiled WASM asset');

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

async function readInbox(page) {
  return page.evaluate(() => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains('inbox_conversations')) { db.close(); reject(new Error('Inbox object store is missing')); return; }
      const transaction = db.transaction('inbox_conversations', 'readonly');
      const read = transaction.objectStore('inbox_conversations').getAll();
      read.onerror = () => { db.close(); reject(new Error(String(read.error))); };
      transaction.oncomplete = () => { db.close(); resolve(read.result); };
      transaction.onabort = () => { db.close(); reject(new Error(String(transaction.error))); };
    };
  }));
}

async function seedPagination(page, name) {
  // Synthetic writes are confined to this isolated temporary origin, after the
  // import/draft/triage flows have already exercised the real UI writer.
  return page.evaluate(({ name }) => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      const transaction = db.transaction('inbox_conversations', 'readwrite');
      const store = transaction.objectStore('inbox_conversations');
      const now = Date.now() - 86_400_000;
      for (let index = 0; index < 205; index++) {
        const suffix = String(index).padStart(3, '0');
        const id = `inbox-browser-${name}-${suffix}`;
        store.put({ id, source: 'import', account_label: '', external_id: null, subject: `Bulk fixture ${suffix}`, participants: [{ id: `${id}-person`, display_name: `Bulk sender ${suffix}`, address: '', person_id: null }], messages: [{ id: `${id}-message`, external_id: null, direction: 'incoming', sender_name: `Bulk sender ${suffix}`, body: `Bulk body ${suffix}. ${'bounded preview '.repeat(20)}${index === 204 ? 'BULK_DEEP_NEEDLE_全履歴' : ''}`, sent_at_ms: now - index * 1000, send_state: 'received', error: null }], unread: false, archived: false, snoozed_until_ms: null, labels: [], created_at_ms: now - index * 1000, updated_at_ms: now - index * 1000 });
      }
      transaction.oncomplete = () => { db.close(); resolve(205); };
      transaction.onabort = () => { db.close(); reject(new Error(String(transaction.error))); };
      transaction.onerror = () => { db.close(); reject(new Error(String(transaction.error))); };
    };
  }), { name });
}

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
        return json({ error: 'Inbox browser verification rejects all external writes.' }, 500);
      }
      if (url.pathname.startsWith('/api/') && loopback(url.hostname)) {
        if (method === 'OPTIONS') return json({});
        state.mockedReads.push({ method, path: url.pathname });
        if (url.pathname.endsWith('/stream')) return route.continue({ url: `${origin}/__fixture-stream` });
        if (url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        if (url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        if (/\/(meetings|people|credentials|tasks|threads|skills|models)$/.test(url.pathname)) return json([]);
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
    const rows = () => page.locator('[data-inbox-row]');
    const filter = name => page.getByRole('group', { name: 'Inbox filter', exact: true }).getByRole('button', { name, exact: true });
    const back = () => page.getByRole('button', { name: 'Back to Inbox', exact: true });
    const search = () => page.getByRole('searchbox', { name: 'Search inbox', exact: true });
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
      await search().waitFor();
      await dismissSidebar();
      await page.getByText('Loading conversations…', { exact: true }).waitFor({ state: 'hidden' });
    }
    async function detail() {
      await page.getByRole('textbox', { name: 'Reply draft', exact: true }).waitFor();
      await back().waitFor();
    }
    async function returnToList() { await back().click(); await ready(); }
    async function openOnlyRow() { await rows().first().click(); await detail(); }
    async function persisted(predicate, label) {
      const deadline = Date.now() + 8000;
      let records = [];
      do {
        records = await readInbox(page);
        if (predicate(records)) { check(true, label, { count: records.length }); return records; }
        await sleep(80);
      } while (Date.now() < deadline);
      check(false, label, { records });
      return records;
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
          // A viewport-fixed dialog escapes the workspace's overflow clip.
          // Keep its own scroll clips, then let the real browser hit test below
          // determine whether the candidate point is covered or clipped.
          if (style.position === 'fixed') break;
        }
        const hit = document.elementFromPoint((left + right) / 2, (top + bottom) / 2);
        return { rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, visibleWidth: right - left, visibleHeight: bottom - top, hit: Boolean(hit && (hit === element || element.contains(hit))), fontSize: Number.parseFloat(getComputedStyle(element).fontSize) };
      });
      check(evidence.visibleWidth > 0 && evidence.visibleHeight > 0 && evidence.hit, `${label} is reachable without clipping or an overlay`, evidence);
      if (cell.hasTouch) check(evidence.rect.width >= 43.9 && evidence.rect.height >= 43.9, `${label} has a 44px touch target`, evidence.rect);
      return evidence;
    }
    async function searchGeometry(label) {
      const evidence = await search().evaluate(input => {
        const toolbar = input.parentElement.parentElement;
        const style = getComputedStyle(toolbar);
        const row = toolbar.getBoundingClientRect(), box = input.getBoundingClientRect();
        const filters = toolbar.querySelector('[aria-label="Inbox filter"]').getBoundingClientRect();
        const paddingLeft = Number.parseFloat(style.paddingLeft) || 0, paddingRight = Number.parseFloat(style.paddingRight) || 0;
        return { toolbar: { x: row.x, width: row.width }, input: { x: box.x, y: box.y, width: box.width, height: box.height }, availableWidth: toolbar.clientWidth - paddingLeft - paddingRight, expectedLeft: row.left + paddingLeft, filtersTop: filters.top, inputBottom: box.bottom };
      });
      check(evidence.input.width >= evidence.availableWidth - 1 && Math.abs(evidence.input.x - evidence.expectedLeft) <= 1 && evidence.filtersTop >= evidence.inputBottom - 1, `${label} search occupies its available toolbar row`, evidence);
    }
    async function historyGeometry() {
      const history = page.locator('[data-inbox-history]');
      await history.waitFor();
      check(await history.getAttribute('aria-label') === 'Conversation messages', 'message history has an explicit accessible region name');
      const evidence = await history.evaluate(history => {
        const inner = history.firstElementChild, article = history.querySelector('article');
        const sender = article.querySelector('p'), body = article.lastElementChild;
        const hs = getComputedStyle(history), innerStyle = getComputedStyle(inner), articleStyle = getComputedStyle(article), senderStyle = getComputedStyle(sender), bodyStyle = getComputedStyle(body);
        const px = value => Number.parseFloat(value) || 0;
        const lineHeight = style => px(style.lineHeight) || px(style.fontSize) * 1.2;
        const senderLine = lineHeight(senderStyle), bodyLine = lineHeight(bodyStyle);
        const verticalPadding = [hs, innerStyle, articleStyle].reduce((sum, style) => sum + px(style.paddingTop) + px(style.paddingBottom), 0);
        const bodyMargins = px(bodyStyle.marginTop) + px(bodyStyle.marginBottom);
        const minimumFromText = senderLine + bodyLine * 3 + verticalPadding + bodyMargins;
        const minimumDesignHeight = px(getComputedStyle(document.documentElement).fontSize) * 8;
        const box = history.getBoundingClientRect();
        let left = Math.max(0, box.left), right = Math.min(innerWidth, box.right), top = Math.max(0, box.top), bottom = Math.min(innerHeight, box.bottom);
        for (let parent = history.parentElement; parent; parent = parent.parentElement) {
          const style = getComputedStyle(parent), clip = parent.getBoundingClientRect();
          if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) { left = Math.max(left, clip.left); right = Math.min(right, clip.right); }
          if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) { top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom); }
          if (style.position === 'fixed') break;
        }
        const text = body.firstChild;
        const range = document.createRange();
        range.setStart(text, 0);
        range.setEnd(text, Math.min(24, text.textContent.indexOf('\n') >= 0 ? text.textContent.indexOf('\n') : text.textContent.length));
        const first = [...range.getClientRects()].find(rect => rect.width > 0 && rect.height > 0);
        const point = first && { x: (first.left + first.right) / 2, y: (first.top + first.bottom) / 2 };
        const hit = point && document.elementFromPoint(point.x, point.y);
        const firstBodyLine = first && { x: first.x, y: first.y, width: first.width, height: first.height };
        return { box: { x: box.x, y: box.y, width: box.width, height: box.height }, clientHeight: history.clientHeight, scrollHeight: history.scrollHeight, senderLine, bodyLine, verticalPadding, bodyMargins, minimumFromText, minimumDesignHeight, expectedHeight: Math.max(minimumFromText, minimumDesignHeight), visibleHistoryHeight: Math.max(0, bottom - top), firstBodyLine, initialBodyVisible: Boolean(first && first.left >= left - 1 && first.right <= right + 1 && first.top >= top - 1 && first.bottom <= bottom + 1 && hit && (hit === body || body.contains(hit))) };
      });
      check(evidence.clientHeight >= evidence.expectedHeight - 1, 'message history fits a sender line, three body lines, and computed vertical padding', evidence);
      check(evidence.initialBodyVisible, 'initial message body text is visible before scrolling to controls', evidence);
    }
    async function layout(label, scope = main) {
      const evidence = await scope.evaluate(element => {
        const rect = element.getBoundingClientRect();
        const targets = [element, ...element.querySelectorAll('article, section[aria-label="Conversations"], [data-inbox-history], form, div.overflow-auto, div.overflow-y-auto')];
        return { viewport: { width: innerWidth, height: innerHeight }, document: { width: document.documentElement.clientWidth, scrollWidth: document.documentElement.scrollWidth }, bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }, overflowing: targets.filter(node => {
          const style = getComputedStyle(node), box = node.getBoundingClientRect();
          return box.width > 0 && box.height > 0 && (box.left < rect.left - 1 || box.right > rect.right + 1 || (node.scrollWidth > node.clientWidth + 1 && !['hidden', 'clip'].includes(style.overflowX)));
        }).map(node => ({ tag: node.tagName, class: node.className, clientWidth: node.clientWidth, scrollWidth: node.scrollWidth, text: node.textContent.slice(0, 100) })) };
      });
      state.layout.push({ label, ...evidence });
      check(evidence.document.scrollWidth <= evidence.document.width + 1 && evidence.bounds.x >= -1 && evidence.bounds.x + evidence.bounds.width <= cell.width + 1 && evidence.overflowing.length === 0, `${label} has no horizontal overflow`, evidence);
    }
    try {
      await page.goto(`${origin}/#/inbox`);
      await ready();
      await page.getByText('Your inbox is clear.', { exact: true }).waitFor();
      check((await readInbox(page)).length === 0, 'fresh browser context starts with empty real IndexedDB');
      check(await page.evaluate(() => matchMedia('(pointer: coarse)').matches) === cell.hasTouch, 'pointer mode matches the verification cell');
      state.stage = 'empty filters';
      const emptyMessages = { Inbox: 'Your inbox is clear.', Unread: 'No unread conversations.', Snoozed: 'No snoozed conversations.', Archived: 'No archived conversations.', All: 'No conversations yet.' };
      for (const [name, message] of Object.entries(emptyMessages)) {
        await filter(name).click();
        await page.getByText(message, { exact: true }).waitFor();
        check(await filter(name).getAttribute('aria-pressed') === 'true' && await rows().count() === 0, `${name} filter has its own empty state and selected state`);
        await reachable(filter(name), `${name} filter`);
      }
      await search().fill('Definitely absent Ω');
      await page.getByText('No matching conversations.', { exact: true }).waitFor();
      check(!await page.getByText('No conversations yet.', { exact: true }).isVisible(), 'search no-results differs from an empty filtered view');
      await search().fill('');
      await filter('Inbox').click();
      await reachable(search(), 'Inbox search');
      await searchGeometry('Empty Inbox');
      await reachable(page.getByRole('button', { name: 'Import conversation', exact: true }), 'Import conversation action');
      await layout('empty Inbox');
      await snapshot('empty');

      state.stage = 'import validation';
      await page.getByRole('button', { name: 'Import conversation', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: 'Import conversation', exact: true });
      await dialog.waitFor();
      const sender = dialog.getByRole('textbox', { name: 'Sender', exact: true });
      const subject = dialog.getByRole('textbox', { name: 'Subject', exact: true });
      const message = dialog.getByRole('textbox', { name: 'Message', exact: true });
      const submit = dialog.getByRole('button', { name: 'Import', exact: true });
      await submit.click();
      await dialog.getByRole('alert').filter({ hasText: 'A sender and message are required.' }).waitFor();
      check((await readInbox(page)).length === 0, 'empty import is rejected without creating a conversation');
      const unicodeSender = `Zoë 林 🌿 ${'Longsender'.repeat(7)}`;
      const unicodeSubject = `Équipe 東京 — café ✓ ${'Longsubject'.repeat(11)}`;
      const deepNeedle = 'NEEDLE_Ω_全本文_σ';
      const literalMarkup = '<img src="https://inbox-fixture.invalid/tracker"><script>throw new Error("inert import")</script>';
      const longBody = `Bonjour 👩🏽‍💻 — première ligne\n${'Résumé 東京, Unicode persists. '.repeat(160)}\n${'unbroken'.repeat(250)}\n${literalMarkup}\n尾部 ${deepNeedle}`;
      await dialog.getByRole('button', { name: 'Slack', exact: true }).click();
      await sender.fill(unicodeSender);
      await subject.fill(unicodeSubject);
      await message.fill('   \n ');
      await submit.click();
      check(await sender.inputValue() === unicodeSender && await subject.inputValue() === unicodeSubject && await message.inputValue() === '   \n ', 'failed import retains sender, subject, and whitespace message');
      check(await dialog.getByRole('button', { name: 'Slack', exact: true }).getAttribute('aria-pressed') === 'true', 'failed import retains the chosen provider');
      await sender.fill('  ');
      await message.fill(longBody);
      await submit.click();
      check(await message.inputValue() === longBody && (await readInbox(page)).length === 0, 'blank sender is rejected while the complete long Unicode message is retained');
      await sender.fill(unicodeSender);
      for (const name of ['Email', 'Slack', 'Discord', 'iMessage', 'X', 'Imported']) await reachable(dialog.getByRole('button', { name, exact: true }), `${name} import provider`);
      for (const [locator, label] of [[sender, 'Sender input'], [subject, 'Subject input'], [message, 'Message input'], [dialog.getByRole('button', { name: 'Cancel', exact: true }), 'Import Cancel'], [submit, 'Import submit']]) await reachable(locator, label);
      await layout('import dialog with long input', dialog);
      await snapshot('import-validation');
      await submit.focus();
      await page.keyboard.press('Tab');
      check(await dialog.evaluate(element => element.contains(document.activeElement)), 'import dialog traps keyboard focus');
      await page.keyboard.press('Escape');
      await dialog.waitFor({ state: 'hidden' });
      await page.getByRole('button', { name: 'Import conversation', exact: true }).click();
      await dialog.waitFor();
      check(await sender.inputValue() === unicodeSender && await subject.inputValue() === unicodeSubject && await message.inputValue() === longBody, 'closing and reopening import retains the local form input');
      await submit.click();
      await detail();
      const recordsAfterImport = await persisted(records => records.length === 1 && records[0].messages[0].body === longBody && records[0].source === 'slack', 'actual import form persists the complete Unicode message and provider to IndexedDB');
      const importedId = recordsAfterImport[0]?.id;
      assert(importedId, 'The actual import did not create an IDB conversation');
      const selectedRecord = records => records.find(record => record.id === importedId);
      await page.getByRole('heading', { name: unicodeSubject, exact: true }).waitFor();
      check(await page.locator('article').first().innerText().then(text => text.includes(longBody)), 'detail renders the full imported history including content beyond the preview');
      check(await page.locator('article img, article script').count() === 0, 'imported markup is inert text');

      state.stage = 'detail layout';
      await historyGeometry();
      await snapshot('long-detail');
      for (const name of ['Back to Inbox', 'Mark unread', 'Tomorrow', 'Important', 'Archive', 'Ask agent', 'Delete', 'Linked person', 'Save draft', 'Log reply']) await reachable(page.getByRole('button', { name, exact: true }), `Detail ${name}`);
      const draft = page.getByRole('textbox', { name: 'Reply draft', exact: true });
      const draftEvidence = await reachable(draft, 'Reply draft');
      if (cell.hasTouch) check(draftEvidence.fontSize >= 16, 'touch reply editor uses at least 16px text', { fontSize: draftEvidence.fontSize });
      await layout('detail with long Unicode and unbroken content');
      await snapshot('detail-controls');

      state.stage = 'full-text search and rows';
      await returnToList();
      await filter('All').click();
      await search().fill(deepNeedle.toLowerCase());
      await rows().first().waitFor();
      check(await rows().count() === 1, 'full-text search matches a case-insensitive Unicode needle beyond the 160-character preview');
      check((await page.getByRole('status', { name: 'Inbox counts', exact: true }).innerText()).includes('1 open'), 'compact counts include the imported open conversation');
      const rowText = await rows().first().innerText();
      check(rowText.includes(unicodeSender) && rowText.includes(unicodeSubject) && rowText.includes('Slack import') && rowText.includes('Bonjour'), 'row displays sender, subject, message preview, and provider source', { rowText });
      check(!rowText.includes(deepNeedle) && rowText.length < 550, 'row preview stays bounded while search uses the complete message', { rowLength: rowText.length });
      check(rowText.includes('Read conversation:'), 'opening an imported conversation exposes its read state in the row');
      await reachable(rows().first(), 'Inbox conversation row');
      const dateTitle = await rows().first().locator('[title]').getAttribute('title');
      check(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(dateTitle ?? '') && /\d{2}:\d{2}/.test(rowText), 'row has a visible message time and full date tooltip', { dateTitle });
      await searchGeometry('Searched Inbox');
      await layout('searched Inbox row');
      await snapshot('search-row');
      await openOnlyRow();
      await returnToList();
      check(await search().inputValue() === deepNeedle.toLowerCase() && await filter('All').getAttribute('aria-pressed') === 'true', 'returning from detail retains both the search and selected filter');
      await search().fill(unicodeSender.toUpperCase());
      check(await rows().count() === 1, 'full-text search includes Unicode participant names');
      await search().fill(unicodeSubject.toUpperCase());
      check(await rows().count() === 1, 'full-text search includes Unicode subjects');
      await search().fill('absent adversarial search term');
      await page.getByText('No matching conversations.', { exact: true }).waitFor();
      check(await rows().count() === 0, 'populated Inbox shows a distinct no-results search state');
      await search().fill('');
      await filter('Inbox').click();

      state.stage = 'read and unread';
      await openOnlyRow();
      await page.getByRole('button', { name: 'Mark unread', exact: true }).click();
      await persisted(records => selectedRecord(records)?.unread === true, 'Mark unread persists through the real IndexedDB writer');
      await page.getByRole('button', { name: 'Mark read', exact: true }).click();
      await persisted(records => selectedRecord(records)?.unread === false, 'Mark read explicitly persists read state');
      await page.getByRole('button', { name: 'Mark unread', exact: true }).click();
      await persisted(records => selectedRecord(records)?.unread === true, 'Mark unread can restore unread state after explicitly marking read');
      await returnToList();
      check((await rows().first().innerText()).includes('Unread conversation:'), 'unread row exposes an explicit accessible unread state');
      check((await page.getByRole('status', { name: 'Inbox counts', exact: true }).innerText()).includes('1 unread'), 'marking unread immediately updates compact counts');
      await filter('Unread').click();
      check(await rows().count() === 1, 'Unread filter includes the unread conversation');
      await openOnlyRow();
      await persisted(records => selectedRecord(records)?.unread === false, 'opening an unread row marks it read and persists the change');
      await returnToList();
      await page.getByText('No unread conversations.', { exact: true }).waitFor();
      check(await filter('Unread').getAttribute('aria-pressed') === 'true', 'Unread selection survives detail navigation when the row becomes read');

      state.stage = 'snooze and archive';
      await filter('Inbox').click();
      await openOnlyRow();
      await page.getByRole('button', { name: 'Tomorrow', exact: true }).click();
      await page.getByRole('button', { name: 'Unsnooze', exact: true }).waitFor();
      await persisted(records => selectedRecord(records)?.snoozed_until_ms > Date.now() + 82_800_000, 'Tomorrow persists a future snooze time');
      await returnToList();
      await page.getByText('Your inbox is clear.', { exact: true }).waitFor();
      check((await page.getByRole('status', { name: 'Inbox counts', exact: true }).innerText()).includes('1 snoozed'), 'snoozing immediately updates compact counts');
      await filter('Snoozed').click();
      check(await rows().count() === 1, 'snoozed conversation leaves Inbox and appears in Snoozed');
      await openOnlyRow();
      await page.getByRole('button', { name: 'Unsnooze', exact: true }).click();
      await persisted(records => selectedRecord(records)?.snoozed_until_ms === null, 'Unsnooze persists removal of the snooze');
      await returnToList();
      await page.getByText('No snoozed conversations.', { exact: true }).waitFor();
      await filter('Inbox').click();
      await openOnlyRow();
      await page.getByRole('button', { name: 'Archive', exact: true }).click();
      await page.getByRole('button', { name: 'Unarchive', exact: true }).waitFor();
      await persisted(records => selectedRecord(records)?.archived === true && selectedRecord(records)?.unread === false, 'Archive persists archived and read state');
      await returnToList();
      await page.getByText('Your inbox is clear.', { exact: true }).waitFor();
      await filter('Archived').click();
      check(await rows().count() === 1, 'archived conversation leaves Inbox and appears in Archived');
      await openOnlyRow();
      await page.getByRole('button', { name: 'Unarchive', exact: true }).click();
      await persisted(records => selectedRecord(records)?.archived === false, 'Unarchive persists restoration');
      await page.getByRole('button', { name: 'Important', exact: true }).click();
      await persisted(records => selectedRecord(records)?.labels.includes('important'), 'Important persists its label');
      await returnToList();
      await page.getByText('No archived conversations.', { exact: true }).waitFor();
      await filter('All').click();
      check((await rows().first().innerText()).includes('Important'), 'important state is visible on the row');
      check(await rows().first().locator('[title]').getAttribute('title') === dateTitle, 'triage changes retain the original message timestamp on the row');

      state.stage = 'draft and local reply';
      await openOnlyRow();
      await page.getByRole('button', { name: 'Unstar', exact: true }).click();
      await persisted(records => !selectedRecord(records)?.labels.includes('important'), 'Unstar durably removes the importance label');
      const logReply = page.getByRole('button', { name: 'Log reply', exact: true });
      check(await logReply.isDisabled(), 'empty local reply cannot be logged');
      await draft.fill('  \n ');
      check(await logReply.isDisabled(), 'whitespace-only local reply cannot be logged');
      const draftBody = `Local reply only — merci 林 👩🏽‍💻\n${'Owner draft keeps Unicode. '.repeat(35)}DRAFT_SEARCH_Δ`;
      await draft.fill(draftBody);
      await page.getByRole('button', { name: 'Save draft', exact: true }).click();
      await persisted(records => selectedRecord(records)?.messages.some(message => message.send_state === 'draft' && message.body === draftBody), 'Save draft persists the complete local draft in IndexedDB');
      await returnToList();
      check(await rows().first().locator('[title]').getAttribute('title') === dateTitle, 'saving a draft retains the displayed received message date');
      check((await rows().first().innerText()).includes('Bonjour') && !(await rows().first().innerText()).includes('Local reply only'), 'saved draft keeps the received message preview intact');
      await search().fill('draft_search_δ');
      check(await rows().count() === 1, 'full-text search reaches a saved draft beyond its hidden preview');
      await openOnlyRow();
      check(await draft.inputValue() === draftBody, 'saved draft survives navigation away and reopening');
      await draft.scrollIntoViewIfNeeded();
      await snapshot('saved-draft');
      await page.reload();
      await detail();
      await dismissSidebar();
      await persisted(records => Boolean(selectedRecord(records)), 'conversation remains in IndexedDB after a page reload');
      check(await draft.inputValue() === draftBody, 'saved reply draft reloads into the real detail editor');
      check(await page.locator('article').count() === 1, 'saved draft stays out of the logged message history');
      await logReply.click();
      await persisted(records => selectedRecord(records)?.messages.filter(message => message.direction === 'outgoing' && message.send_state !== 'draft' && message.body === draftBody).length === 1, 'Log reply records exactly one outgoing reply locally');
      check(await draft.inputValue() === '' && await logReply.isDisabled(), 'logging clears the editor and disables an empty repeat submission');
      check((await page.locator('article').allTextContents()).some(text => text.includes(draftBody)), 'logged local reply appears in conversation history');
      await page.reload();
      await detail();
      await dismissSidebar();
      check((await page.locator('article').allTextContents()).some(text => text.includes(draftBody)) && await draft.inputValue() === '', 'logged reply survives reload with no pending draft');
      await page.locator('article').last().locator('p').first().scrollIntoViewIfNeeded();
      await snapshot('logged-reply-detail');
      await returnToList();
      await filter('All').click();
      await search().fill(deepNeedle);
      check(await rows().count() === 1, 'search still finds an older incoming body after the row preview changes to the reply');
      check((await page.getByRole('status', { name: 'Inbox counts', exact: true }).innerText()).includes('0 need reply'), 'logging a local reply immediately updates the need-reply count');
      check(!(await rows().first().innerText()).includes(deepNeedle), 'older full-history search result does not depend on the last-message preview');
      await search().fill('draft_search_δ');
      check(await rows().count() === 1, 'full-text search includes the complete logged local reply');
      await snapshot('logged-reply-row');

      state.stage = 'bounded pagination';
      await search().fill('');
      check(await seedPagination(page, cell.name) === 205, 'pagination fixture adds 205 synthetic records only after real form and writer checks');
      await page.reload();
      await ready();
      await filter('All').click();
      await search().fill('Bulk fixture');
      const pagination = page.getByRole('navigation', { name: 'Inbox pages', exact: true });
      await pagination.getByText('1 to 100 of 205', { exact: true }).waitFor();
      check(await rows().count() === 100, 'first page mounts at most 100 rows for 205 matching conversations');
      check(await pagination.getByRole('button', { name: 'Previous', exact: true }).isDisabled(), 'first page disables Previous');
      await reachable(pagination.getByRole('button', { name: 'Next', exact: true }), 'Inbox Next page');
      await pagination.getByRole('button', { name: 'Next', exact: true }).click();
      await pagination.getByText('101 to 200 of 205', { exact: true }).waitFor();
      check(await rows().count() === 100 && (await rows().first().innerText()).includes('Bulk fixture 100'), 'second page shows the correct next 100 distinct rows');
      await pagination.getByRole('button', { name: 'Next', exact: true }).click();
      await pagination.getByText('201 to 205 of 205', { exact: true }).waitFor();
      check(await rows().count() === 5 && await pagination.getByRole('button', { name: 'Next', exact: true }).isDisabled(), 'last page contains the remaining five rows and disables Next');
      await reachable(pagination.getByRole('button', { name: 'Previous', exact: true }), 'Inbox Previous page');
      await layout('paginated Inbox');
      await snapshot('pagination-last');
      await pagination.getByRole('button', { name: 'Previous', exact: true }).click();
      await pagination.getByText('101 to 200 of 205', { exact: true }).waitFor();
      await search().fill('BULK_DEEP_NEEDLE_全履歴');
      await rows().first().waitFor();
      check(await rows().count() === 1 && (await rows().first().innerText()).includes('Bulk fixture 204'), 'search reaches complete history in a record beyond the previously mounted pages');
      check(!await pagination.isVisible(), 'a narrow search resets pagination and hides unnecessary page controls');
      await openOnlyRow();
      await returnToList();
      check(await search().inputValue() === 'BULK_DEEP_NEEDLE_全履歴' && await filter('All').getAttribute('aria-pressed') === 'true', 'query and All filter survive detail navigation for an off-page record');
      await search().fill('Bulk fixture');
      await pagination.getByText('1 to 100 of 205', { exact: true }).waitFor();
      await pagination.getByRole('button', { name: 'Next', exact: true }).click();
      await filter('Unread').click();
      await page.getByText('No matching conversations.', { exact: true }).waitFor();
      check(await rows().count() === 0 && !await pagination.isVisible(), 'filter change from a later page resets the page and produces a reachable empty result');
      await filter('All').click();
      await pagination.getByText('1 to 100 of 205', { exact: true }).waitFor();
      check(await rows().count() === 100, 'returning to All starts at the first page after filter change');
      await search().fill('Bulk fixture 204');
      await rows().first().waitFor();
      const oldDateTitle = await rows().first().locator('[title]').getAttribute('title');
      check(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(oldDateTitle ?? '') && /\d{4}-\d{2}-\d{2}/.test(await rows().first().innerText()), 'older rows show a calendar date plus the full message timestamp', { oldDateTitle });
      await layout('final filtered Inbox');
      await snapshot('final');
      check((await readInbox(page)).length === 206, 'all UI-created and pagination records persist in real IndexedDB');
    } catch (error) {
      check(false, 'workflow completed without an unexpected browser interaction failure', { error: error.stack ?? String(error) });
      await snapshot('failure').catch(() => {});
    } finally {
      state.stage = 'runtime and privacy';
      check(state.runtimeErrors.length === 0, 'no JavaScript/WASM runtime errors', state.runtimeErrors);
      check(state.blockedWrites.length === 0, 'import, triage, save draft, and log reply attempt no external writes', state.blockedWrites);
      check(state.unexpectedRequests.length === 0, 'imported text causes no unexpected external content requests', state.unexpectedRequests);
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
