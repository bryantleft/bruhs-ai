// Actual compiled WASM; synthetic server-cache/connection/sync transport only.
// Browser downloads save only explicit synthetic attachment bytes to QA files.
// No real provider, OAuth, credentials, or email send.
// Require an explicit NEW capture; this script never builds or installs.
// NO_COLOR=true node bruhs-web/tests/connected_mail_browser.mjs /fresh/dist
// Set BRUHS_CONNECTED_MAIL_BUILD_RECEIPT and BRUHS_CONNECTED_MAIL_BUILD_INPUTS
// to the capture receipts for SHA/mtime/source verification before launch.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const self = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(self), '../..');
assert(process.argv[2], 'Explicit fresh connected-mail WASM capture required; never use the old Today/base capture');
const dist = path.resolve(process.argv[2]);
const mailProvider = process.env.BRUHS_CONNECTED_MAIL_PROVIDER ?? 'gmail';
assert(['gmail', 'outlook'].includes(mailProvider), 'Choose gmail or outlook fixture mode');
const providerName = mailProvider === 'gmail' ? 'Gmail' : 'Outlook Mail';
const syncLabel = mailProvider === 'gmail' ? 'Sync Gmail' : 'Sync Outlook';
const otherSyncLabel = mailProvider === 'gmail' ? 'Sync Outlook' : 'Sync Gmail';
const syncRoute = '/api/mail/' + mailProvider + '/sync';
const attachmentPrefix = mailProvider === 'gmail' ? '/api/mail' : '/api/mail/outlook';
const startedAt = new Date().toISOString();
const output = path.resolve(process.env.BRUHS_CONNECTED_MAIL_QA_OUTPUT ?? path.join(root, 'target/inbox-ui-qa/connected-mail-browser', startedAt.replace(/[:.]/g, '-')));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sourceFiles = [
  'bruhs-web/src/components/connected_inbox.rs', 'bruhs-web/src/components/inbox_page.rs',
  'bruhs-web/src/state/mail.rs', 'bruhs-web/src/state/mod.rs', 'bruhs-web/src/state/workspace.rs',
  'bruhs-web/src/api/mail.rs', 'bruhs-web/src/api/http.rs', 'bruhs-web/src/api.rs',
  'bruhs-web/src/app.rs', 'bruhs-web/src/components/mod.rs', 'bruhs-web/src/components/settings_page.rs',
  'bruhs-web/src/components/service_connections.rs', 'bruhs-web/src/components/task_pagination.rs',
  'bruhs-web/src/storage.rs', 'bruhs-web/src/ui/button.rs', 'bruhs-web/src/ui/input.rs',
  'bruhs-web/src/ui/textarea.rs', 'bruhs-web/src/ui/dialog.rs', 'bruhs-web/src/ui/page_header.rs',
  'bruhs-web/index.html', 'bruhs-web/tailwind.css', 'bruhs-models/src/mail.rs',
  'bruhs-models/src/connections.rs', 'bruhs-models/src/mail/compose.rs', 'bruhs-models/src/models/inbox.rs',
  'bruhs-models/src/models/inbox/directory.rs',
];
const sources = await Promise.all(sourceFiles.map(async file => ({ file, modifiedMs: (await stat(path.join(root, file))).mtimeMs, sha256: hash(await readFile(path.join(root, file))) })));
const index = await stat(path.join(dist, 'index.html'));
assert(index.mtimeMs >= Math.max(...sources.map(source => source.modifiedMs)), 'Refuse stale connected-mail assets: obtain a successful fresh production capture');
async function filesIn(directory, prefix = '') {
  const entries = await readdir(directory, { withFileTypes: true });
  return (await Promise.all(entries.map(entry => entry.isDirectory() ? filesIn(path.join(directory, entry.name), prefix + entry.name + '/') : [prefix + entry.name]))).flat().sort();
}
const assetNames = await filesIn(dist);
assert(assetNames.some(name => name.endsWith('.wasm')), 'Capture has no compiled WASM');
const assets = await Promise.all(assetNames.map(async file => ({ file, sha256: hash(await readFile(path.join(dist, file))), mtime_ns: String((await stat(path.join(dist, file), { bigint: true })).mtimeNs) })));
async function receipt(env) {
  if (!process.env[env]) return null;
  const file = path.resolve(process.env[env]), bytes = await readFile(file);
  // Preserve nanosecond integers, which exceed JavaScript's safe-integer range.
  const value = JSON.parse(bytes.toString().replace(/("mtime_ns"\s*:\s*)(\d+)/g, '$1"$2"'));
  return { file, sha256: hash(bytes), value };
}
const build = await receipt('BRUHS_CONNECTED_MAIL_BUILD_RECEIPT');
const buildInputs = await receipt('BRUHS_CONNECTED_MAIL_BUILD_INPUTS');
if (build) {
  assert(build.value.exitCode === 0, 'Capture receipt is not successful');
  assert(path.resolve(build.value.dist) === dist, 'Capture receipt names another dist');
  const expectedIndex = build.value.indexSha256 ?? build.value.assets?.['index.html']?.sha256;
  assert(expectedIndex === hash(await readFile(path.join(dist, 'index.html'))), 'Capture index hash mismatch');
  assert.deepEqual(Object.keys(build.value.assets).sort(), assetNames, 'Capture file set differs from receipt');
  for (const asset of assets) {
    const expected = build.value.assets[asset.file];
    assert(asset.sha256 === expected.sha256, 'Capture hash mismatch: ' + asset.file);
    assert(asset.mtime_ns === String(expected.mtime_ns), 'Capture mtime mismatch: ' + asset.file);
  }
}
if (buildInputs) {
  for (const source of sources) {
    assert(buildInputs.value.sources[source.file]?.sha256 === source.sha256, 'Source differs from capture: ' + source.file);
  }
}
await mkdir(output, { recursive: true });
const report = {
  startedAt, dist, output, provider: mailProvider, sources, assets, harnessSha256: hash(await readFile(self)),
  receipts: { build: build && { file: build.file, sha256: build.sha256 }, inputs: buildInputs && { file: buildInputs.file, sha256: buildInputs.sha256 } },
  freshness: { indexModifiedMs: index.mtimeMs, newestSourceMs: Math.max(...sources.map(source => source.modifiedMs)) },
  scope: 'Actual compiled connected Email UI, native browser Blob/download behavior and actual isolated imported Inbox IndexedDB. All cache, account, provider-sync and binary attachment responses are explicit synthetic fixtures. Downloaded QA files are checked by exact bytes, SHA and suggested name. No real server/provider attachment retrieval, OAuth, credential/keychain or mail send acceptance.',
  limits: ['Synthetic transport does not establish actual provider/server search/sync/attachment correctness', 'No native/device, live provider, mailbox send, performance or physical offline-device acceptance', 'Network-unavailable and HTTP-error paths are deliberately injected and exactly classified', '20s/180s native timer registration is passively observed; full deadline expiration is not waited', 'Largest successful fixture is8MiB+17bytes;64MiB+1metadata is refused without a binary read', 'No builds, installs, application edits or Git operations'],
  cells: [], checks: [], fixturePageSize: 100,
};
let playwright;
if (process.env.BRUHS_PLAYWRIGHT_PATH) playwright = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH);
else {
  try { playwright = await import('playwright'); } catch {
    for (const entry of await readdir(path.join(homedir(), '.npm/_npx'))) {
      try { playwright = await import(pathToFileURL(path.join(homedir(), '.npm/_npx', entry, 'node_modules/playwright/index.mjs')).href); break; } catch {}
    }
  }
}
assert(playwright, 'Use an existing Playwright installation; no dependencies are installed');
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (!['GET', 'HEAD'].includes(request.method)) { response.writeHead(403); response.end(); return; }
  if (url.pathname === '/__fixture-stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Access-Control-Allow-Origin': '*' });
    response.write(': connected-mail synthetic fixture\n\n');
    const timer = setInterval(() => response.write(': keepalive\n\n'), 1000);
    request.on('close', () => clearInterval(timer));
    return;
  }
  const file = path.resolve(dist, '.' + (url.pathname === '/' ? '/index.html' : decodeURIComponent(url.pathname)));
  if (!file.startsWith(dist + path.sep)) { response.writeHead(403); response.end(); return; }
  try {
    const bytes = await readFile(file);
    response.writeHead(200, { 'Content-Type': { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.png': 'image/png' }[path.extname(file)] ?? 'application/octet-stream' });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = 'http://127.0.0.1:' + server.address().port;
report.origin = origin;
const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.BRUHS_CHROME ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : undefined) });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const clone = value => structuredClone(value);
const loopback = hostname => ['127.0.0.1', 'localhost', '[::1]'].includes(hostname);
const queryNeedle = 'CAFÉ 世界 & page=9/#?% + exact';
const literalMarkup = '<script>globalThis.__connectedMailExecuted=true</script><img src="https://connected-mail-fixture.invalid/tracker" onerror="globalThis.__connectedMailExecuted=true">';
const baseTime = Date.parse('2026-01-12T18:20:00Z');
const cells = [{ name: 'desktop', width: 1280, height: 800, hasTouch: false }, { name: 'compact', width: 640, height: 480, hasTouch: false }, { name: 'touch', width: 390, height: 844, hasTouch: true }];
const kinds = ['browse-import-history', 'errors-sync-stop-resume', 'stale-host-responses', 'offline-empty-accounts', 'attachments-download-success', 'attachments-held-route', 'attachments-errors-retry'];
const binaryBytes = Buffer.from(Array.from({ length: 1041 }, (_, n) => (n * 131 + 17) & 255));
const largeBytes = Buffer.alloc((8 << 20) + 17);
for (let n = 0; n < largeBytes.length; n++) largeBytes[n] = (n * 131 + 23) & 255;
const htmlBytes = Buffer.from('<!doctype html><meta charset="utf-8"><title>Untrusted fixture</title><script>globalThis.__attachmentPayloadExecuted=true;fetch("https://attachment-fixture.invalid/script")</script><img src="https://attachment-fixture.invalid/image"><p>café 東京</p>\u0000', 'utf8');
const attachmentFixtures = [
  { id: 'attachment/σ &?regular', file_name: '../folder\\résumé-東京.bin', expected_name: 'résumé-東京.bin', bytes: binaryBytes, inline: false },
  { id: 'inline/empty 林 &?', file_name: '', expected_name: 'attachment', bytes: Buffer.alloc(0), inline: true },
  { id: 'large/8MiB Σ &?', file_name: 'large-Σ.bin', expected_name: 'large-Σ.bin', bytes: largeBytes, inline: false },
  { id: 'html/東京 &?', file_name: 'literal-東京.html', expected_name: 'literal-東京.html', bytes: htmlBytes, inline: false },
  { id: 'over/64MiB 林', file_name: 'oversize.bin', expected_name: 'oversize.bin', bytes: null, size: (64 << 20) + 1, inline: false },
];
if (mailProvider === 'outlook') {
  const bytes = Buffer.from('From: sender@example.test\r\nSubject: Outlook MIME export\r\n\r\nCafé 林\r\n', 'utf8');
  attachmentFixtures.push({ id: 'item/export 林 &?', file_name: 'attached-message.eml', expected_name: 'attached-message.eml', bytes, size: 1000, exportSize: bytes.length, inline: false });
  kinds.push('attachments-outlook-capabilities');
}

function createFixture(id) {
  const fixture = {
    host: 'qa host/A 東京 &?', primary: 'mail/box 東京 &?', secondary: 'empty/box &?', prefix: 'HOST_A',
    revision: 4, count: 205, emptyAccounts: false, connectionId: 'current-gmail-generation-2',
    connectionState: 'connected', cachedConnectionId: 'cached-old-generation-1',
    syncPlan: [], attachmentPlan: [], attachmentSpecs: id.includes('-attachments-') ? attachmentFixtures.map(spec => ({ ...spec })) : null,
    holds: [], committedPages: [], failNext: null, nextHold: [], teardown: false,
  };
  fixture.accountLabel = 'Zoë 林 ' + 'longlabel'.repeat(7) + '@example.test';
  fixture.mailboxes = () => fixture.emptyAccounts ? [] : [
    { mailbox_id: fixture.primary, provider: mailProvider, last_connection_id: fixture.cachedConnectionId, account_label: fixture.accountLabel, revision: fixture.revision, threads: fixture.count, drafts: 0 },
    { mailbox_id: fixture.secondary, provider: mailProvider, last_connection_id: 'old-empty-generation', account_label: 'Second saved account 東京', revision: 1, threads: 0, drafts: 0 },
  ];
  fixture.snapshot = () => ({ host_id: fixture.host, mailboxes: fixture.mailboxes() });
  fixture.thread = (mailbox, external, prefix = fixture.prefix) => {
    const messages = Array.from({ length: 7 }, (_, n) => ({
      id: id + '-message-' + n, external_id: external + '/message/' + n,
      direction: n === 6 ? 'outgoing' : 'incoming', sender_name: n === 6 ? 'You' : 'Actual sender Zoë 林',
      body: 'HISTORY_PART_' + n + '\n' + ('Résumé 東京 plain text. '.repeat(n === 6 ? 30 : 4)) + '\n' + 'unbroken'.repeat(75) + '\n' + literalMarkup + (n === 0 ? '\nDEEP_ONLY ' + queryNeedle : ''),
      sent_at_ms: baseTime + n * 60_000, send_state: n === 6 ? 'sent' : 'received', error: null,
    }));
    return {
      id: id + '-canonical-' + external, source: 'email', account_label: 'Mailbox owner must not replace the real sender',
      external_id: external, subject: prefix + ' saved thread 東京 — ' + 'longsubject'.repeat(12),
      participants: [{ id: 'wrong-owner', display_name: 'Mailbox owner', address: 'owner@example.test', person_id: null }],
      messages, unread: true, archived: false, snoozed_until_ms: null, labels: [], created_at_ms: baseTime, updated_at_ms: baseTime + 360_000,
      connected_mail: {
        provider: mailProvider, mailbox_id: mailbox, external_thread_id: external,
        messages: messages.map((message, n) => ({
          id: message.id, external_id: message.external_id, label_ids: ['INBOX'],
          envelope: {
            from: n === 6 ? [{ name: 'Saved outgoing owner', address: 'owner@example.test' }] : [{ name: 'Actual sender Zoë 林 ' + 'longsender'.repeat(5), address: 'real+東京@example.test' }],
            to: [{ name: 'Recipient équipe', address: 'recipient@example.test' }],
            cc: [{ name: 'Cc café', address: 'copy@example.test' }], bcc: [], reply_to: [],
            subject: prefix + ' saved thread 東京', message_id: '<synthetic-' + n + '@example.test>',
            in_reply_to: [], references: [], headers: [{ name: 'X-Synthetic-QA', value: id }],
          },
          attachments: fixture.attachmentSpecs && n === 6
            ? fixture.attachmentSpecs.map((spec, index) => ({ id: spec.id, part_id: '1.' + index, external_id: spec.reference || (spec.inline && mailProvider === 'gmail') ? null : 'opaque-provider-' + index, file_name: spec.file_name, content_type: index === 3 ? 'text/html' : 'application/octet-stream', size_bytes: spec.size ?? spec.bytes.length, content_id: spec.inline ? 'inline-synthetic' : null, inline: spec.inline }))
            : [{ id: 'attachment-' + n, part_id: '1.2', external_id: 'opaque-attachment-' + n, file_name: 'invoice-東京-<script>literal</script>-' + n + '.pdf', content_type: 'application/pdf', size_bytes: 4096 + n, content_id: null, inline: false }],
          body_from_html: true,
        })),
      },
    };
  };
  fixture.records = mailbox => mailbox !== fixture.primary ? [] : Array.from({ length: fixture.count }, (_, n) => ({
    external: 'thread/σ &?' + String(n).padStart(3, '0'), unread: n % 2 === 0, archived: n % 3 === 0, snoozed: n % 5 === 0, index: n,
  }));
  fixture.matches = (mailbox, query, filter) => fixture.records(mailbox).filter(record => {
    const text = ('actual sender zoë 林 ' + fixture.prefix + ' saved thread résumé 東京 ' + queryNeedle).toLowerCase();
    if (query.trim() && !text.includes(query.trim().toLowerCase())) return false;
    return filter === 'all' || (filter === 'unread' ? record.unread : filter === 'archived' ? record.archived : filter === 'snoozed' ? record.snoozed : !record.archived && !record.snoozed);
  });
  fixture.browse = (mailbox, query, filter, requested) => {
    const records = fixture.matches(mailbox, query, filter), page = Math.min(requested, Math.max(0, Math.ceil(records.length / 100) - 1));
    return {
      host_id: fixture.host, mailbox_id: mailbox, revision: fixture.revision, page, total_matches: records.length,
      rows: records.slice(page * 100, (page + 1) * 100).map(record => ({
        external_id: record.external,
        card: { id: id + '-row-' + record.index, source: 'email', subject: fixture.prefix + ' subject ' + record.index + ' 東京 — ' + 'longsubject'.repeat(6), sender: 'Actual sender Zoë 林 ' + 'longsender'.repeat(5), preview: 'Newest outgoing preview; the search needle is only in the oldest saved body.', unread: record.unread, important: false, message_at_ms: baseTime + record.index * 60_000 },
      })),
    };
  };
  fixture.hold = (kind, label, predicate = () => true) => fixture.nextHold.push({ kind, label, predicate });
  fixture.release = async label => {
    const hold = fixture.holds.find(item => item.label === label);
    assert(hold, 'Fixture did not receive held request: ' + label);
    hold.release();
    await hold.done;
  };
  fixture.syncResult = (threads, caughtUp) => ({ host_id: fixture.host, mailbox_id: fixture.primary, revision: fixture.revision + 1, threads_updated: threads, full_scan_complete: caughtUp, caught_up: caughtUp });
  return fixture;
}

async function readImported(page) {
  return page.evaluate(() => new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('Actual app must create its schema')); };
    open.onerror = () => reject(new Error(String(open.error)));
    open.onsuccess = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains('inbox_conversations')) { db.close(); reject(new Error('Missing actual Inbox store')); return; }
      const tx = db.transaction('inbox_conversations', 'readonly'), request = tx.objectStore('inbox_conversations').getAll();
      tx.oncomplete = () => { db.close(); resolve(request.result); };
      tx.onabort = () => { db.close(); reject(new Error(String(tx.error))); };
    };
  }));
}

try {
  for (const cell of cells) for (const kind of kinds) {
    const id = cell.name + '-' + kind, fixture = createFixture(id);
    const state = { id, kind, ...cell, stage: 'setup', screenshots: [], requests: [], responses: [], runtimeErrors: [], consoleErrors: [], requestFailures: [], unexpected: [], blockedWrites: [], expectedErrors: [], rejectedExportHeaders: [], snapshots: [], downloads: [], blobSnapshots: [], readCancellations: [], browserResponses: [] };
    const requestSequences = new WeakMap();
    report.cells.push(state);
    const check = (passed, label, expected = undefined, actual = undefined) => {
      const entry = { case: id, stage: state.stage, label, passed: Boolean(passed), ...(expected === undefined ? {} : { expected }), ...(actual === undefined ? {} : { actual }) };
      report.checks.push(entry);
      if (!passed) console.error(JSON.stringify(entry));
      return Boolean(passed);
    };
    const stage = label => { state.stage = label; console.log(JSON.stringify({ case: id, stage: label })); };
    const until = async (predicate, label) => {
      const deadline = Date.now() + 12_000;
      while (Date.now() < deadline) { if (await predicate()) return; await sleep(30); }
      throw new Error('Timed out: ' + label);
    };
    const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, hasTouch: cell.hasTouch, acceptDownloads: true, serviceWorkers: 'block', locale: 'en-US', timezoneId: 'America/Los_Angeles', reducedMotion: kind === 'browse-import-history' ? 'reduce' : 'no-preference' });
    await context.addInitScript(() => {
      const reads = globalThis.__mailReadCancellationQa = { requests: [] };
      const nativeFetch = globalThis.fetch;
      globalThis.fetch = function (...args) {
        const input = args[0], options = args[1];
        const url = typeof input === 'string' ? input : input instanceof URL ? String(input) : input.url;
        const method = String(options?.method ?? input?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && /^\/api\/mail\/mailboxes\/[^/]+\/threads(?:\/[^/]+)?$/.test(new URL(url, location.href).pathname)) {
          const signal = options?.signal ?? input?.signal;
          const record = { id: reads.requests.length, url, method, at: performance.now(), signalPresent: signal instanceof AbortSignal, initiallyAborted: signal?.aborted ?? null };
          reads.requests.push(record);
          if (signal) signal.addEventListener('abort', () => { record.aborted = true; record.abortedAt = performance.now(); }, { once: true });
        }
        return Reflect.apply(nativeFetch, this, args);
      };
      globalThis.__connectedMailExecuted = false;
      globalThis.__attachmentPayloadExecuted = false;
      const ledger = globalThis.__mailDownloadQa = { created: [], revoked: [], clicks: [], removed: [], timers: [] };
      const create = URL.createObjectURL, revoke = URL.revokeObjectURL;
      URL.createObjectURL = function (value) {
        const result = Reflect.apply(create, this, [value]);
        ledger.created.push({ url: result, size: value instanceof Blob ? value.size : null, type: value instanceof Blob ? value.type : null });
        return result;
      };
      URL.revokeObjectURL = function (value) { ledger.revoked.push(String(value)); return Reflect.apply(revoke, this, [value]); };
      const click = HTMLAnchorElement.prototype.click;
      HTMLAnchorElement.prototype.click = function (...args) {
        if (this.href.startsWith('blob:')) ledger.clicks.push({ url: this.href, name: this.download, connected: this.isConnected, hidden: this.hidden });
        return Reflect.apply(click, this, args);
      };
      new MutationObserver(records => {
        for (const record of records) for (const node of record.removedNodes) {
          if (node instanceof HTMLAnchorElement && node.href.startsWith('blob:') && node.hasAttribute('download')) ledger.removed.push(node.href);
        }
      }).observe(document, { childList: true, subtree: true });
      const timeout = globalThis.setTimeout;
      globalThis.setTimeout = function (callback, delay, ...args) {
        if (delay === 20_000 || delay === 180_000) ledger.timers.push(delay);
        return Reflect.apply(timeout, globalThis, [callback, delay, ...args]);
      };
    });
    context.on('page', page => {
      page.on('response', response => state.browserResponses.push({ url: response.url(), requestSequence: requestSequences.get(response.request()), status: response.status() }));
      page.on('pageerror', error => state.runtimeErrors.push(error.stack ?? error.message));
      page.on('console', message => { if (message.type() === 'error') state.consoleErrors.push({ text: message.text(), url: message.location().url }); });
      page.on('requestfailed', request => state.requestFailures.push({ url: request.url(), error: request.failure()?.errorText, requestSequence: requestSequences.get(request) }));
    });
    await context.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url()), method = request.method();
      const json = async (value, status = 200) => {
        if (status >= 400) state.expectedErrors.push({ url: request.url(), status });
        if (method === 'POST' && url.pathname === syncRoute && status < 400) {
          fixture.revision = value.revision;
          fixture.committedPages.push({ host_id: value.host_id, mailbox_id: value.mailbox_id, revision: value.revision, caught_up: value.caught_up });
        }
        await route.fulfill({ status, contentType: 'application/json', headers: { 'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'GET, HEAD, OPTIONS, POST', 'Access-Control-Allow-Private-Network': 'true', 'Cache-Control': 'no-store' }, body: JSON.stringify(value) });
      };
      if (url.pathname.startsWith('/api/') && loopback(url.hostname)) {
        if (method === 'OPTIONS') return json({});
        const record = { sequence: state.requests.length, method, url: request.url(), pathname: url.pathname, query: Object.fromEntries(url.searchParams), body: request.postDataJSON?.() ?? null };
        state.requests.push(record);
        requestSequences.set(request, record.sequence);
        let type, value, status = 200;
        const browse = url.pathname.match(/^\/api\/mail\/mailboxes\/([^/]+)\/threads$/);
        const thread = url.pathname.match(/^\/api\/mail\/mailboxes\/([^/]+)\/threads\/([^/]+)$/);
        const attachment = url.pathname.match(new RegExp(`^${attachmentPrefix}/mailboxes/([^/]+)/threads/([^/]+)/attachments/([^/]+)$`));
        if (method === 'GET' && attachment) {
          record.type = 'attachment';
          const mailbox = decodeURIComponent(attachment[1]), external = decodeURIComponent(attachment[2]), attachmentId = decodeURIComponent(attachment[3]);
          check(mailbox === fixture.primary && external === fixture.activeThread, 'attachment read uses the selected canonical mailbox/thread identifiers', { mailbox: fixture.primary, thread: fixture.activeThread }, { mailbox, external });
          check(Object.keys(record.query).sort().join(',') === 'connection_id,host_id' && record.query.connection_id === fixture.connectionId && record.query.connection_id !== fixture.cachedConnectionId, 'attachment read resolves the current broker generation with exact encoded query boundaries', fixture.connectionId, record.query);
          check(record.url.includes('/attachments/' + encodeURIComponent(attachmentId) + '?') && record.url.includes('connection_id=' + encodeURIComponent(fixture.connectionId)), 'attachment id and current generation are encoded as opaque values', attachmentId, record.url);
          if (record.query.host_id !== fixture.host) return json({ error: 'Synthetic attachment server identity changed. Refresh saved mail.' }, 409);
          const spec = fixture.attachmentSpecs?.find(item => item.id === attachmentId);
          const next = fixture.attachmentPlan.shift();
          if (!spec || !next || next.id !== attachmentId) {
            state.unexpected.push({ ...record, reason: 'No explicit synthetic attachment response was authorized' });
            return json({ error: 'Unplanned attachment read' }, 403);
          }
          if (next.network) {
            state.expectedErrors.push({ url: request.url(), network: 'ERR_INTERNET_DISCONNECTED' });
            return route.abort('internetdisconnected');
          }
          if (next.status) return json({ error: next.message }, next.status);
          const body = next.truncated ? spec.bytes.subarray(0, Math.max(0, spec.bytes.length - 1)) : spec.bytes;
          assert(body, 'Oversized metadata must be rejected before a binary read');
          const summary = { binary: true, attachmentId, bytes: body.length, sha256: hash(body), expectedBytes: spec.size ?? spec.bytes.length, contentType: 'application/octet-stream' };
          const exportHeaders = spec.exportSize == null ? {} : { 'X-Bruhs-Mail-Export-Size': String(next.exportSizeOverride ?? spec.exportSize), 'Access-Control-Expose-Headers': 'X-Bruhs-Mail-Export-Size' };
          if (mailProvider === 'outlook' && kind === 'attachments-outlook-capabilities' && spec.exportSize != null && ['bad-size', String((64 << 20) + 1)].includes(next.exportSizeOverride)) {
            state.rejectedExportHeaders.push({ sequence: record.sequence, url: request.url(), header: next.exportSizeOverride });
          }
          const deliver = () => route.fulfill({ status: 200, headers: { 'Access-Control-Allow-Origin': origin, 'Content-Type': 'application/octet-stream', 'Content-Disposition': 'attachment; filename=synthetic.bin', 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-store', 'Content-Length': String(body.length), ...exportHeaders }, body });
          if (next.hold) {
            let release, finish;
            const gate = new Promise(resolve => { release = resolve; }), done = new Promise(resolve => { finish = resolve; });
            fixture.holds.push({ label: next.hold, record, value: summary, release, done, released: false });
            await gate;
            fixture.holds.find(item => item.label === next.hold).released = true;
            try { if (fixture.teardown) await route.abort(); else await deliver(); }
            catch (error) { if (!fixture.teardown) state.unexpected.push({ type: 'held-binary-route-error', error: String(error) }); }
            finish();
          } else await deliver();
          state.responses.push({ sequence: record.sequence, type: 'attachment', status: 200, value: summary });
          return;
        }
        if (method === 'POST' && url.pathname === syncRoute) {
          type = 'sync';
          const body = record.body;
          check(body && Object.keys(body).sort().join(',') === 'connection_id,host_id,mailbox_id,restart', 'sync uses the explicit canonical request fields', ['connection_id', 'host_id', 'mailbox_id', 'restart'], body);
          check(body?.connection_id === fixture.connectionId && body?.connection_id !== fixture.cachedConnectionId, 'sync resolves the current mail authorization generation rather than cached last_connection_id', fixture.connectionId, body?.connection_id);
          if (body?.host_id !== fixture.host) { status = 409; value = { error: 'Synthetic server identity changed. Refresh saved mail.' }; }
          else {
            const next = fixture.syncPlan.shift();
            if (!next) { state.unexpected.push(record); return json({ error: 'No synthetic sync page was authorized' }, 500); }
            status = next.status ?? 200;
            value = next.error ? { error: next.error } : fixture.syncResult(next.threads ?? 1, next.caughtUp ?? true);
            if (next.hold) fixture.nextHold.unshift({ kind: 'sync', label: next.hold, predicate: () => true });
          }
        } else if (method === 'GET' && url.pathname === '/api/mail') { type = 'snapshot'; value = fixture.snapshot(); }
        else if (method === 'GET' && browse) {
          type = 'browse';
          const mailbox = decodeURIComponent(browse[1]), host = url.searchParams.get('host_id');
          if (host !== fixture.host) { status = 409; value = { error: 'Synthetic read belongs to another server identity.' }; }
          else value = fixture.browse(mailbox, url.searchParams.get('query') ?? '', url.searchParams.get('filter') ?? 'inbox', Number(url.searchParams.get('page') ?? 0));
        } else if (method === 'GET' && thread) {
          type = 'thread';
          if (url.searchParams.get('host_id') !== fixture.host) { status = 409; value = { error: 'Synthetic thread belongs to another server identity.' }; }
          else { fixture.activeThread = decodeURIComponent(thread[2]); value = fixture.thread(decodeURIComponent(thread[1]), fixture.activeThread); }
        }
        else if (method === 'GET' && /^\/api\/connections\/(google|microsoft|github|gmail|outlook)$/.test(url.pathname)) {
          const provider = url.pathname.split('/').at(-1);
          type = provider === mailProvider ? 'connection' : 'other-connection';
          value = { id: provider === mailProvider && fixture.connectionState === 'connected' ? fixture.connectionId : null, provider, configured: true, account: provider === mailProvider ? 'synthetic-owner@example.test' : null, scopes: [], state: provider === mailProvider ? fixture.connectionState : 'disconnected' };
        } else if (method === 'GET' && url.pathname === '/api/events/stream') return route.continue({ url: origin + '/__fixture-stream' });
        else if (method === 'GET' && url.pathname === '/api/vault') return json({ entries: [], bruhs: [], one_password: false });
        else if (method === 'GET' && ['/api/terminal/sessions', '/api/people', '/api/tasks', '/api/threads', '/api/meetings', '/api/models', '/api/skills', '/api/calendars', '/api/calendar/google/calendars'].includes(url.pathname)) return json([]);
        else if (method === 'GET' && ['/api/agent/auth/claude', '/api/agent/auth/codex', '/api/capture/status', '/api/people/versions', '/api/updates'].includes(url.pathname)) return json({});
        else if (method === 'GET' && url.pathname === '/api/calendar/google/state') return json({ connected: false, email: null, calendars: [], last_synced_at: null, last_error: null });
        else if (method === 'GET' && url.pathname === '/api/reactive') return json({ agents: [], sources: [], subscriptions: [], events: [], runs: [], proposals: [], source_statuses: [] });
        else if (method === 'GET' && url.pathname === '/api/weekly-reports') return json({ presets: [], reports: [], next_before: null });
        else {
          if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) state.blockedWrites.push(record); else state.unexpected.push(record);
          return json({ error: 'Connected mail QA rejects this unplanned API action' }, 403);
        }
        record.type = type;
        const failure = fixture.failNext?.kind === type ? fixture.failNext : null;
        if (failure) {
          fixture.failNext = null;
          if (failure.network) { state.expectedErrors.push({ url: request.url(), network: 'ERR_INTERNET_DISCONNECTED' }); return route.abort('internetdisconnected'); }
          status = failure.status; value = { error: failure.message };
        }
        const wanted = fixture.nextHold.findIndex(item => item.kind === type && item.predicate(record));
        if (wanted >= 0) {
          const spec = fixture.nextHold.splice(wanted, 1)[0];
          let release, finish;
          const gate = new Promise(resolve => { release = resolve; }), done = new Promise(resolve => { finish = resolve; });
          fixture.holds.push({ label: spec.label, record, value: clone(value), release, done, released: false });
          await gate;
          fixture.holds.find(item => item.label === spec.label).released = true;
          try { await json(value, status); } catch (error) { if (!fixture.teardown) state.unexpected.push({ type: 'held-route-error', error: String(error) }); }
          finish();
          return;
        }
        state.responses.push({ sequence: record.sequence, type, status, value: clone(value) });
        return json(value, status);
      }
      if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) { state.blockedWrites.push({ method, url: request.url() }); return route.abort(); }
      if (url.origin === origin) return route.continue();
      state.unexpected.push({ method, url: request.url() });
      return route.abort();
    });
    const page = await context.newPage();
    const downloadJobs = [];
    page.on('download', download => {
      const index = state.downloads.length;
      const record = { index, url: download.url(), suggestedName: download.suggestedFilename(), file: path.join(output, id + '-download-' + index + '.bin') };
      state.downloads.push(record);
      downloadJobs.push((async () => {
        try {
          await download.saveAs(record.file);
          record.failure = await download.failure();
          const bytes = await readFile(record.file);
          record.bytes = bytes.length; record.sha256 = hash(bytes);
        } catch (error) { record.error = String(error); }
      })());
    });
    page.setDefaultTimeout(12_000);
    const email = page.locator('[data-connected-inbox]');
    const rows = () => email.locator('[data-mail-row]');
    const search = () => email.getByRole('searchbox', { name: 'Search email', exact: true });
    const filter = name => email.getByRole('group', { name: 'Email filter', exact: true }).getByRole('button', { name, exact: true });
    const header = name => email.locator('header').getByRole('button', { name, exact: true });
    const accounts = () => email.getByRole('group', { name: 'Saved mail accounts', exact: true }).getByRole('button');
    const requestCount = type => state.requests.filter(request => request.type === type).length;
    const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const screenshot = async name => { const file = id + '-' + name + '.png'; await page.screenshot({ path: path.join(output, file) }); state.screenshots.push(file); };
    const held = label => until(() => fixture.holds.some(item => item.label === label), 'held ' + label);
    async function release(label) {
      const item = fixture.holds.find(candidate => candidate.label === label);
      assert(item, 'No held response: ' + label);
      const cancelledReadLabels = new Map([['old-query', 'browse'], ['old-account', 'browse'], ['closed-reader', 'thread'], ['old-host-rows', 'browse'], ['old-host-thread', 'thread']]);
      if (cancelledReadLabels.has(label)) {
        const record = item.record;
        const identityValid = kind === 'stale-host-responses' && record.method === 'GET' && record.type === cancelledReadLabels.get(label)
          && state.requests[record.sequence] === record && !state.readCancellations.some(expected => expected.sequence === record.sequence);
        check(identityValid, 'stale read cancellation binds one exact held GET label, URL and sequence', { label, kind: cancelledReadLabels.get(label) }, { label, kind: record.type, sequence: record.sequence, url: record.url });
        assert(identityValid, 'Refuse cancellation classification for a request outside the five explicit held GETs');
        const ordinal = state.requests.filter(candidate => candidate.method === record.method && candidate.url === record.url && candidate.sequence <= record.sequence).length - 1;
        const nativeRead = () => page.evaluate(({ url, method, ordinal }) => globalThis.__mailReadCancellationQa.requests.filter(request => request.url === url && request.method === method)[ordinal] ?? null, { url: record.url, method: record.method, ordinal });
        let observed;
        await until(async () => {
          observed = await nativeRead();
          return observed?.signalPresent && observed.initiallyAborted === false && observed.aborted === true
            && state.requestFailures.some(failure => failure.requestSequence === record.sequence && failure.url === record.url && failure.error === 'net::ERR_ABORTED');
        }, 'original signal and exact held request confirm cancellation before response release: ' + label);
        const failures = state.requestFailures.filter(failure => failure.requestSequence === record.sequence && failure.url === record.url);
        const beforeResponse = state.browserResponses.filter(response => response.requestSequence === record.sequence && response.url === record.url);
        const nativeValid = observed?.url === record.url && observed.method === record.method && observed.signalPresent && observed.initiallyAborted === false && observed.aborted === true && observed.abortedAt >= observed.at && observed.abortedAt - observed.at < 20_000;
        check(nativeValid, 'stale held GET uses its original native AbortSignal before the20s deadline', { label, abortedBeforeDeadline: true }, observed);
        check(failures.length === 1 && failures[0].error === 'net::ERR_ABORTED' && beforeResponse.length === 0, 'stale held GET has exactly one matching canceled browser request and no response before release', { label, failures: 1, responses: 0 }, { failures, beforeResponse });
        assert(nativeValid && failures.length === 1 && failures[0].error === 'net::ERR_ABORTED' && beforeResponse.length === 0, 'Refuse unproved stale-read error classification');
        const verified = { label, sequence: record.sequence, url: record.url, method: record.method, type: record.type, ordinal, nativeRequest: observed, failure: failures[0], verifiedBeforeRelease: true, lateMockFulfillAttempted: true };
        state.readCancellations.push(verified);
        await fixture.release(label);
        await settle();
        verified.released = item.released;
        verified.browserResponsesAfterRelease = state.browserResponses.filter(response => response.requestSequence === record.sequence && response.url === record.url);
        check(item.released && verified.browserResponsesAfterRelease.length === 0, 'late mock response is still attempted after canceled GET and cannot revive the native read', { label, released: true, responses: 0 }, { released: item.released, responses: verified.browserResponsesAfterRelease });
        return;
      }
      const arrival = page.waitForResponse(response => response.url() === item.record.url);
      await fixture.release(label);
      const response = await arrival;
      await response.finished();
      await settle();
    }
    const readyRows = async () => { await until(async () => await email.isVisible() && !await email.getByText('Refreshing saved accounts…', { exact: true }).isVisible() && !await email.getByText('Loading cached email…', { exact: true }).isVisible(), 'mail reads finish'); };
    const syncIdle = async () => { await until(async () => await header(syncLabel).isEnabled() && !await header('Stop sync').isVisible(), 'sync ends'); await readyRows(); };
    async function dismissSidebar() {
      const scrim = page.locator('div[aria-hidden="true"].fixed.inset-0').first();
      if (await scrim.isVisible()) await scrim.click({ position: { x: cell.width - 12, y: 60 } });
    }
    async function start() {
      await page.goto(origin + '/#/inbox');
      await page.getByRole('searchbox', { name: 'Search inbox', exact: true }).waitFor();
      await dismissSidebar();
      await page.getByText('Loading conversations…', { exact: true }).waitFor({ state: 'hidden' });
      check(requestCount('snapshot') === 0 && !state.requests.some(request => request.pathname.startsWith('/api/mail')), 'imported Inbox does not eagerly fetch connected Email');
    }
    async function openEmail() {
      await page.getByRole('button', { name: 'Email', exact: true }).click();
      await email.waitFor();
      await until(() => requestCount('snapshot') > 0, 'lazy account request starts');
      await readyRows();
    }
    async function applySearch(value) {
      await search().fill(value);
      await search().press('Enter');
      await readyRows();
    }
    async function reachable(locator, label, measureTouch = true) {
      await locator.scrollIntoViewIfNeeded();
      const actual = await locator.evaluate(element => {
        const box = element.getBoundingClientRect(), x = Math.max(0, Math.min(innerWidth - 1, box.x + box.width / 2)), y = Math.max(0, Math.min(innerHeight - 1, box.y + box.height / 2)), hit = document.elementFromPoint(x, y), style = getComputedStyle(element);
        return { x: box.x, y: box.y, width: box.width, height: box.height, hit: Boolean(hit && (hit === element || element.contains(hit))), fontFamily: style.fontFamily, fontSize: parseFloat(style.fontSize) };
      });
      check(actual.hit && actual.x >= -1 && actual.x + actual.width <= cell.width + 1 && actual.y >= -1 && actual.y + actual.height <= cell.height + 1, label + ' is scroll-reachable and hit-testable', { width: cell.width, height: cell.height }, actual);
      if (cell.hasTouch && measureTouch) check(actual.width >= 43.9 && actual.height >= 43.9, label + ' has a44px touch target', { minimum: 44 }, actual);
      check(/Monaspace|monospace/i.test(actual.fontFamily), label + ' retains monospace typography', 'Monaspace/monospace', actual.fontFamily);
      return actual;
    }
    async function layout(label) {
      const actual = await email.evaluate(element => {
        const box = element.getBoundingClientRect();
        const overflow = [...element.querySelectorAll('header, form, nav, article, [data-mail-row], [data-mail-message-body], [aria-label="Saved mail accounts"] button')].map(node => {
          const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
          return { tag: node.tagName, text: node.textContent.slice(0, 90), x: rect.x, width: rect.width, clientWidth: node.clientWidth, scrollWidth: node.scrollWidth, overflowX: style.overflowX };
        }).filter(node => node.width > 0 && (node.x < box.x - 1 || node.x + node.width > box.right + 1 || (node.scrollWidth > node.clientWidth + 1 && !['hidden', 'clip'].includes(node.overflowX))));
        return { viewportWidth: innerWidth, documentWidth: document.documentElement.clientWidth, scrollWidth: document.documentElement.scrollWidth, root: { x: box.x, width: box.width }, overflowing: overflow };
      });
      check(actual.scrollWidth <= actual.documentWidth + 1 && actual.root.x >= -1 && actual.root.x + actual.root.width <= cell.width + 1 && actual.overflowing.length === 0, label + ' has no horizontal overflow', { maximumWidth: cell.width, overflowing: [] }, actual);
    }
    async function inert() {
      check(await email.locator('[data-mail-message-body] img, [data-mail-message-body] script, [data-mail-message-body] iframe, [data-mail-message-body] a').count() === 0, 'saved script/image/HTML content remains plain text with no active descendants');
      check(await page.evaluate(() => globalThis.__connectedMailExecuted) === false, 'saved script and image handlers never execute');
    }
    const downloadButtons = () => email.locator('[data-mail-attachment-download]');
    async function scrollReaderToAction(index) {
      const action = downloadButtons().nth(index);
      await action.scrollIntoViewIfNeeded();
      await action.evaluate(element => {
        let scroller = element.parentElement;
        while (scroller && !(/auto|scroll/.test(getComputedStyle(scroller).overflowY) && scroller.scrollHeight > scroller.clientHeight + 1)) scroller = scroller.parentElement;
        if (!scroller) throw new Error('No real scrollable reader ancestor');
        const box = scroller.getBoundingClientRect(), target = element.getBoundingClientRect();
        const maximum = scroller.scrollHeight - scroller.clientHeight;
        scroller.scrollTop = Math.min(maximum, Math.max(Math.min(240, maximum), scroller.scrollTop + target.top - box.top - Math.min(80, scroller.clientHeight / 3)));
      });
      await settle();
    }
    async function feedbackViewport(locator, label) {
      await locator.waitFor();
      await settle();
      const actual = await locator.evaluate(element => {
        const box = element.getBoundingClientRect();
        let left = Math.max(0, box.left), right = Math.min(innerWidth, box.right), top = Math.max(0, box.top), bottom = Math.min(innerHeight, box.bottom);
        for (let parent = element.parentElement; parent; parent = parent.parentElement) {
          const style = getComputedStyle(parent), bounds = parent.getBoundingClientRect();
          if (/auto|scroll|hidden|clip/.test(style.overflowX)) { left = Math.max(left, bounds.left); right = Math.min(right, bounds.right); }
          if (/auto|scroll|hidden|clip/.test(style.overflowY)) { top = Math.max(top, bounds.top); bottom = Math.min(bottom, bounds.bottom); }
          if (style.position === 'fixed') break;
        }
        const hit = right > left && bottom > top ? document.elementFromPoint((left + right) / 2, (top + bottom) / 2) : null;
        const region = element.closest('[data-mail-feedback]'), statusBox = region?.getBoundingClientRect();
        let reader = document.querySelector('[data-mail-attachment-download]')?.parentElement;
        while (reader && !(/auto|scroll/.test(getComputedStyle(reader).overflowY) && reader.scrollHeight > reader.clientHeight + 1)) reader = reader.parentElement;
        return {
          text: element.textContent, box: { x: box.x, y: box.y, width: box.width, height: box.height },
          visibleWidth: Math.max(0, right - left), visibleHeight: Math.max(0, bottom - top),
          hit: Boolean(hit && (hit === element || element.contains(hit))),
          readerScrollTop: reader?.scrollTop ?? 0, readerContainsFeedback: Boolean(reader?.contains(element)),
          region: region && { height: statusBox.height, maxHeight: getComputedStyle(region).maxHeight, overflowY: getComputedStyle(region).overflowY, tabindex: region.tabIndex },
        };
      });
      check(actual.visibleWidth >= actual.box.width - 1 && actual.visibleHeight >= actual.box.height - 1 && actual.hit && actual.readerScrollTop >= 120, label + ' is viewport-visible and hit-testable while reader remains scrolled, without scrolling feedback', { fullyVisible: true, hit: true, minimumReaderScroll: 120 }, actual);
      check(actual.region && actual.region.height <= 128.1 && actual.region.maxHeight === '128px' && actual.region.overflowY === 'auto' && actual.region.tabindex === 0 && !actual.readerContainsFeedback, label + ' uses the bounded focusable status region outside the reader scroller', { maximumHeight: 128, tabindex: 0, outsideReader: true }, actual.region);
    }
    async function blobSnapshot(label) {
      await settle();
      const value = await page.evaluate(() => {
        const ledger = globalThis.__mailDownloadQa;
        return {
          ...ledger,
          active: ledger.created.filter(item => !ledger.revoked.includes(item.url)),
          remainingAnchors: document.querySelectorAll('a[download][href^="blob:"]').length,
          payloadExecuted: globalThis.__attachmentPayloadExecuted,
          pagesUrl: location.href,
        };
      });
      state.blobSnapshots.push({ label, ...value });
      return value;
    }
    async function saveProof(before, spec) {
      await until(() => state.downloads.length > before, 'real browser download starts');
      await downloadJobs[before];
      const actual = state.downloads[before], bytes = actual.error ? null : await readFile(actual.file);
      check(state.downloads.length === before + 1 && !actual.error && actual.failure === null, 'one actual browser attachment download completes without a browser failure', { downloads: before + 1, failure: null }, actual);
      check(bytes?.equals(spec.bytes) && actual.bytes === spec.bytes.length && actual.sha256 === hash(spec.bytes), 'downloaded file preserves exact binary bytes and SHA256', { bytes: spec.bytes.length, sha256: hash(spec.bytes) }, { bytes: actual.bytes, sha256: actual.sha256 });
      check(actual.suggestedName === spec.expected_name && actual.url.startsWith('blob:'), 'browser uses the sanitized suggested filename and a local Blob URL', { name: spec.expected_name, scheme: 'blob:' }, { name: actual.suggestedName, url: actual.url });
      const snapshot = await blobSnapshot('after-download-' + before);
      check(snapshot.active.length === 0 && snapshot.remainingAnchors === 0 && snapshot.clicks.length === snapshot.created.length && snapshot.created.every(item => snapshot.removed.includes(item.url)), 'download removes its temporary anchor and revokes every created Blob URL', { active: 0, anchors: 0 }, snapshot);
      check(snapshot.created.at(-1)?.size === spec.bytes.length && snapshot.created.at(-1)?.type === 'application/octet-stream', 'attachment stays a native browser Blob with the exact octet-stream size', { size: spec.bytes.length, type: 'application/octet-stream' }, snapshot.created.at(-1));
      check(snapshot.clicks.at(-1)?.connected === true && snapshot.clicks.at(-1)?.hidden === true && snapshot.clicks.at(-1)?.name === spec.expected_name, 'the real hidden download anchor is connected when clicked and names the sanitized file', { connected: true, hidden: true, name: spec.expected_name }, snapshot.clicks.at(-1));
      check(snapshot.payloadExecuted === false && await page.evaluate(() => globalThis.__connectedMailExecuted) === false && context.pages().length === 1, 'binary and HTML attachment payloads never execute or open another page');
      return actual;
    }
    async function downloadSuccess(index, label) {
      const spec = fixture.attachmentSpecs[index], before = state.downloads.length, connections = requestCount('connection'), reads = requestCount('attachment');
      fixture.connectionId = 'download/current generation/' + label + '/林 &?';
      fixture.attachmentPlan.push({ id: spec.id });
      await scrollReaderToAction(index);
      await downloadButtons().nth(index).click();
      await saveProof(before, spec);
      await until(() => downloadButtons().nth(index).isEnabled(), 'download control is idle');
      check(requestCount('connection') === connections + 1 && requestCount('attachment') === reads + 1, 'explicit download fetches one current connection and one binary response', { connections: connections + 1, reads: reads + 1 }, { connections: requestCount('connection'), reads: requestCount('attachment') });
      check(await email.locator('[data-mail-notice]').innerText() === 'Attachment download started. Check your browser\'s downloads for completion.' && !await email.locator('[data-mail-download-error]').isVisible(), 'successful download status says started/check browser completion and never claims saved');
      await feedbackViewport(email.locator('[data-mail-notice]'), 'Download success feedback');
    }
    try {
      await start();
      if (kind === 'browse-import-history') {
        stage('real imported form and saved draft before Email');
        const localNeedle = 'LOCAL_RETAINED_全履歴';
        const localDraft = 'Saved local draft only — café 林; never send.';
        await page.getByRole('button', { name: 'Import conversation', exact: true }).click();
        const dialog = page.getByRole('dialog', { name: 'Import conversation', exact: true });
        await dialog.getByRole('textbox', { name: 'Sender', exact: true }).fill('Imported Zoë 林');
        await dialog.getByRole('textbox', { name: 'Subject', exact: true }).fill('Retained local import 東京');
        await dialog.getByRole('textbox', { name: 'Message', exact: true }).fill('Local-only body ' + localNeedle);
        await dialog.getByRole('button', { name: 'Import', exact: true }).click();
        const draft = page.getByRole('textbox', { name: 'Reply draft', exact: true });
        await draft.waitFor();
        await draft.fill(localDraft);
        await page.getByRole('button', { name: 'Save draft', exact: true }).click();
        await until(async () => (await readImported(page)).some(item => item.messages.some(message => message.body === localDraft && message.send_state === 'draft')), 'real saved draft persists');
        state.snapshots.push({ label: 'local-draft-before-email', records: await readImported(page) });
        await page.getByRole('button', { name: 'Back to Inbox', exact: true }).click();
        await page.getByRole('searchbox', { name: 'Search inbox', exact: true }).fill(localNeedle);
        await page.getByRole('group', { name: 'Inbox filter', exact: true }).getByRole('button', { name: 'All', exact: true }).click();
        check(!state.requests.some(request => request.pathname.startsWith('/api/mail')), 'local import, search and draft do not grant access or trigger connected-mail reads');
        await openEmail();
        stage('saved account list, lazy fetch and typography');
        check(requestCount('snapshot') === 1 && await accounts().count() === 2, 'Email lazily fetches and renders both saved accounts', 2, await accounts().count());
        check(await email.getByRole('heading', { name: 'Email', exact: true }).isVisible(), 'compiled connected Email view mounts');
        for (const name of ['Imported conversations', 'Connections', 'Refresh', syncLabel, otherSyncLabel]) await reachable(header(name), 'Email ' + name);
        await reachable(accounts().nth(0), 'Long saved account label');
        await reachable(accounts().nth(1), 'Second saved account');
        const countSpan = await accounts().nth(0).locator(':scope > span').last().evaluate(element => {
          const range = document.createRange(); range.selectNodeContents(element);
          return { text: element.textContent, whiteSpace: getComputedStyle(element).whiteSpace, lineRects: range.getClientRects().length };
        });
        check(countSpan.text === '205 threads' && countSpan.whiteSpace === 'nowrap' && countSpan.lineRects === 1, 'saved account count stays on one readable line beside the wrapping label', { text: '205 threads', lines: 1 }, countSpan);
        await layout('saved accounts and initial rows');
        await screenshot('accounts-initial');
        await accounts().nth(1).click();
        await readyRows();
        check(await rows().count() === 0 && await email.locator('[data-mail-empty]').innerText() === 'No saved email matches this view. Try another search or choose All.', 'empty saved account differs from an unconnected account');
        await accounts().nth(0).click();
        await readyRows();
        stage('encoded full-history search and all filters');
        const beforeTyping = requestCount('browse');
        await search().fill(queryNeedle);
        await settle();
        check(requestCount('browse') === beforeTyping, 'typing a private query does not submit until Search/Enter');
        await search().press('Enter');
        await readyRows();
        for (const label of ['Inbox', 'Unread', 'Snoozed', 'Archived', 'All']) {
          await filter(label).click();
          await readyRows();
          const request = state.requests.filter(item => item.type === 'browse').at(-1);
          const total = fixture.matches(fixture.primary, queryNeedle, label.toLowerCase()).length;
          check(request.query.query === queryNeedle && request.query.filter === label.toLowerCase() && request.query.page === '0', label + ' submits the complete Unicode/full-text query and resets paging', { query: queryNeedle, filter: label.toLowerCase(), page: '0' }, request.query);
          check(request.url.includes('query=' + encodeURIComponent(queryNeedle)) && request.url.includes('/mail%2Fbox%20%E6%9D%B1%E4%BA%AC%20%26%3F/'), 'query and mailbox are encoded without creating new query/path boundaries', queryNeedle, request.url);
          check(Object.keys(request.query).sort().join(',') === 'filter,host_id,page,query' && request.query.host_id === fixture.host, 'search preserves the explicit server identity and exact query parameter boundaries', fixture.host, request.query);
          check(await filter(label).getAttribute('aria-pressed') === 'true' && await rows().count() === Math.min(100, total), label + ' selected state and bounded result page match the cache contract', Math.min(100, total), await rows().count());
          await reachable(filter(label), label + ' email filter');
        }
        check(!(await rows().first().innerText()).includes(queryNeedle) && await rows().count() === 100, 'full-history response is useful even when its matching oldest body is absent from the preview');
        await reachable(search(), 'Email search input');
        await reachable(email.getByRole('button', { name: 'Search', exact: true }), 'Email Search submit');
        await search().focus();
        await search().press('Tab');
        const focus = await email.getByRole('button', { name: 'Search', exact: true }).evaluate(element => ({ focused: document.activeElement === element, visible: element.matches(':focus-visible'), outline: getComputedStyle(element).outlineWidth }));
        check(focus.focused && focus.visible && parseFloat(focus.outline) >= 2, 'keyboard Search focus has a visible outline', { outlineAtLeast: 2 }, focus);
        const motion = await search().evaluate(element => ({ reduced: matchMedia('(prefers-reduced-motion: reduce)').matches, transition: getComputedStyle(element).transitionDuration, animation: getComputedStyle(element).animationDuration, font: getComputedStyle(element).fontFamily }));
        const duration = text => Math.max(...text.split(',').map(value => parseFloat(value) * (value.trim().endsWith('ms') ? 0.001 : 1)));
        check(motion.reduced && duration(motion.transition) <= 0.00002 && duration(motion.animation) <= 0.00002, 'reduced-motion media preference collapses transitions/animation', { maximumSeconds: 0.00002 }, motion);
        await layout('searched email');
        await screenshot('unicode-search');
        await applySearch('NO_MATCH_AT_ALL_Ω');
        check(await rows().count() === 0 && await email.locator('[data-mail-page]').innerText() === '0 threads', 'empty search uses the zero-result page contract');
        await applySearch(queryNeedle);
        stage('last-page contract and canonical clamped page');
        const next = email.getByRole('navigation', { name: 'Email pages', exact: true }).getByRole('button', { name: 'Next', exact: true });
        const previous = email.getByRole('navigation', { name: 'Email pages', exact: true }).getByRole('button', { name: 'Previous', exact: true });
        check(await email.locator('[data-mail-page]').innerText() === '1 to 100 of 205' && await previous.isDisabled(), 'first page has100 rows and no previous page');
        await next.click(); await readyRows();
        check(await email.locator('[data-mail-page]').innerText() === '101 to 200 of 205' && await rows().count() === 100, 'second page is bounded and reports the correct range');
        await next.click(); await readyRows();
        check(await email.locator('[data-mail-page]').innerText() === '201 to 205 of 205' && await rows().count() === 5 && await next.isDisabled(), 'last page displays its five records and disables Next');
        await reachable(previous, 'Email Previous page');
        await screenshot('last-page');
        fixture.count = 101; fixture.revision++;
        await header('Refresh').click(); await readyRows();
        check(await email.locator('[data-mail-page]').innerText() === '101 to 101 of 101' && await rows().count() === 1 && await next.isDisabled(), 'refresh honors the server-clamped last page after the result set shrinks');
        stage('keyboard reader, real metadata and every saved message');
        const row = rows().first(), rowId = await row.getAttribute('id');
        const rowText = await row.innerText();
        check(rowText.includes('Actual sender Zoë 林') && !rowText.includes('Mailbox owner') && rowText.includes('HOST_A subject') && rowText.includes('Newest outgoing preview'), 'row shows real sender, subject and preview rather than substituting the account label', 'Actual sender Zoë 林', rowText);
        check(await row.locator('[title]').getAttribute('title') === '2026-01-12 12:00', 'row date includes the complete local date/time metadata', '2026-01-12 12:00', await row.locator('[title]').getAttribute('title'));
        check(rowText.startsWith('Read email:') || rowText.startsWith('Unread email:'), 'row exposes explicit read/unread state');
        await reachable(row, 'Saved email row');
        await row.focus(); await row.press('Enter');
        const back = email.getByRole('button', { name: 'Back to email', exact: true });
        await until(() => back.evaluate(element => document.activeElement === element), 'reader keyboard focus moves to Back');
        await email.locator('[data-mail-message-body]').waitFor();
        check(await email.locator('[data-mail-message-index]').innerText() === 'Message 7 of 7', 'saved reader opens at the newest message');
        const request = state.requests.filter(item => item.type === 'thread').at(-1);
        check(request.url.includes('/thread%2F%CF%83%20%26%3F100?') && request.query.host_id === fixture.host, 'saved thread uses encoded external id and explicit host identity', 'thread/σ &?100', request.url);
        const expectedThread = fixture.thread(fixture.primary, 'thread/σ &?100');
        const articleText = await email.getByRole('article', { name: 'Saved message', exact: true }).innerText();
        check(articleText.includes('From: Saved outgoing owner <owner@example.test>') && articleText.includes('To: Recipient équipe <recipient@example.test>') && articleText.includes('Cc: Cc café <copy@example.test>'), 'saved outgoing message renders its actual From/To/Cc envelope metadata', 'actual saved envelope', articleText);
        const metadataText = await email.locator('[data-mail-attachment] p').innerText();
        check(metadataText === 'Attachment: invoice-東京-<script>literal</script>-6.pdf (4102 bytes)' && await email.locator('[data-mail-attachment-download]').isEnabled(), 'attachment metadata stays literal and truthfully states download availability', 'literal filename/size and an enabled explicit Download attachment action', metadataText);
        check(requestCount('attachment') === 0 && state.downloads.length === 0, 'reading saved attachment metadata never automatically fetches or downloads its bytes');
        const older = email.getByRole('button', { name: 'Older message', exact: true }), newer = email.getByRole('button', { name: 'Newer message', exact: true });
        check(await newer.isDisabled(), 'newest message disables Newer');
        await reachable(back, 'Reader Back to email');
        await reachable(older, 'Reader Older message');
        await reachable(newer, 'Reader Newer message');
        await reachable(email.getByRole('button', { name: 'Copy message text', exact: true }), 'Copy plain message text');
        for (let n = 6; n >= 0; n--) {
          check(await email.locator('[data-mail-message-body]').textContent() === expectedThread.messages[n].body, 'saved history message ' + n + ' retains the complete plain text', 'HISTORY_PART_' + n, (await email.locator('[data-mail-message-body]').textContent()).slice(0, 60));
          if (n < 6) check((await email.getByRole('article', { name: 'Saved message', exact: true }).innerText()).includes('From: Actual sender Zoë 林') && (await email.getByRole('article', { name: 'Saved message', exact: true }).innerText()).includes('real+東京@example.test'), 'incoming history retains the real sender envelope instead of the account label');
          await inert();
          if (n) { await older.click(); await email.getByText('Message ' + n + ' of 7', { exact: true }).waitFor(); }
        }
        check(await older.isDisabled(), 'oldest message disables Older');
        await layout('oldest full message and metadata');
        await screenshot('oldest-plain-history');
        for (let n = 1; n < 7; n++) { await newer.click(); await email.getByText('Message ' + (n + 1) + ' of 7', { exact: true }).waitFor(); }
        check(await email.locator('[data-mail-message-body]').textContent() === expectedThread.messages[6].body && await newer.isDisabled(), 'Newer traverses the complete history back to its newest boundary');
        await screenshot('reader-latest');
        await back.focus(); await back.press('Enter');
        await until(() => page.evaluate(expected => document.activeElement?.id === expected, rowId), 'Back restores keyboard focus to the opened row');
        const restoredFocus = await rows().first().evaluate(element => ({ focused: document.activeElement === element, visible: element.matches(':focus-visible'), outline: getComputedStyle(element).outlineWidth }));
        check(restoredFocus.focused && restoredFocus.visible && parseFloat(restoredFocus.outline) >= 2, 'Back restores a visible keyboard focus ring to the originating row', { rowId }, restoredFocus);
        check(await search().inputValue() === queryNeedle && await filter('All').getAttribute('aria-pressed') === 'true' && await email.locator('[data-mail-page]').innerText() === '101 to 101 of 101', 'reader return retains connected query, filter and page');
        stage('imported browse/draft retention and Connections Settings');
        await header('Imported conversations').click();
        check(await page.getByRole('searchbox', { name: 'Search inbox', exact: true }).inputValue() === localNeedle && await page.getByRole('group', { name: 'Inbox filter', exact: true }).getByRole('button', { name: 'All', exact: true }).getAttribute('aria-pressed') === 'true', 'Email navigation retains the imported search and filter');
        await page.locator('[data-inbox-row]').first().click();
        check(await page.getByRole('textbox', { name: 'Reply draft', exact: true }).inputValue() === localDraft, 'Email navigation retains the actual imported draft');
        const saved = await readImported(page);
        check(saved.length === 1 && saved[0].messages.some(message => message.body === localDraft && message.send_state === 'draft'), 'connected reads do not replace local imported records or turn drafts into sends', 1, saved.length);
        state.snapshots.push({ label: 'local-draft-after-email', records: saved });
        await page.getByRole('button', { name: 'Back to Inbox', exact: true }).click();
        await openEmail();
        await header('Connections').click();
        await page.getByRole('heading', { name: 'Settings', exact: true }).waitFor();
        const vault = page.locator('#vault');
        await vault.getByRole('heading', { name: 'Connected accounts', exact: true }).scrollIntoViewIfNeeded();
        check(await vault.getByRole('heading', { name: 'Connected accounts', exact: true }).isVisible(), 'Connections opens Settings with reachable connected-account controls');
        await screenshot('connections-settings');
      } else if (kind === 'errors-sync-stop-resume') {
        await openEmail();
        stage('read error and explicit retry');
        fixture.failNext = { kind: 'browse', status: 503, message: 'Synthetic saved mail is unavailable. Retry Refresh.' };
        await header('Refresh').click(); await readyRows();
        await email.getByRole('alert').filter({ hasText: 'Synthetic saved mail is unavailable. Retry Refresh.' }).waitFor();
        await screenshot('read-error');
        await header('Refresh').click(); await readyRows();
        check(!await email.getByRole('alert').count() && await rows().count() > 0, 'successful explicit Refresh recovers a cache read error');
        stage('failed sync persists through successful cache refresh');
        const snapshotsBefore = requestCount('snapshot');
        fixture.syncPlan.push({ status: 507, error: `Synthetic mailbox quota exhausted. Retry ${syncLabel}.` });
        await header(syncLabel).click(); await syncIdle();
        check(await email.locator('[data-mail-sync-error]').innerText() === `Synthetic mailbox quota exhausted. Retry ${syncLabel}.` && requestCount('snapshot') > snapshotsBefore && await rows().count() > 0, 'failed sync stays visible after its successful account/rows refresh', 'persistent quota error with usable cached rows', await email.locator('[data-mail-sync-error]').innerText());
        check(!await email.locator('[data-mail-notice]').isVisible(), 'failed sync does not leave a false success/in-progress notice');
        await screenshot('sync-error-after-successful-refresh');
        stage('current connection generation and automatic multi-page sync');
        fixture.connectionId = 'current-gmail-generation-3';
        const beforePages = requestCount('sync');
        fixture.syncPlan.push({ threads: 2, caughtUp: false }, { threads: 3, caughtUp: false }, { threads: 4, caughtUp: true });
        await header(syncLabel).click(); await syncIdle();
        check(requestCount('sync') - beforePages === 3 && await email.locator('[data-mail-notice]').innerText() === `${providerName} sync finished. Threads updated: 9.`, 'one explicit sync continues through all server pages and reports their total', 3, requestCount('sync') - beforePages);
        check(!await email.locator('[data-mail-sync-error]').isVisible(), 'successful sync retry clears the prior sync error');
        await screenshot('multi-page-sync-finished');
        stage('Stop waits for the held current page');
        const stopStart = requestCount('sync'), committedBefore = fixture.committedPages.length;
        fixture.syncPlan.push({ threads: 2, caughtUp: false, hold: 'stop-current-page' });
        await header(syncLabel).click(); await held('stop-current-page');
        await reachable(header('Stop sync'), 'Stop sync action');
        await header('Stop sync').click();
        check(await header('Stop sync').isDisabled() && fixture.committedPages.length === committedBefore, 'Stop marks intent while the real current synthetic transport page remains held');
        await screenshot('stop-current-page-held');
        await release('stop-current-page'); await syncIdle();
        check(requestCount('sync') === stopStart + 1 && fixture.committedPages.length === committedBefore + 1 && await email.locator('[data-mail-notice]').innerText() === 'Sync stopped. Saved pages will be used when you resume.', 'Stop saves the held page and dispatches no next page', { requests: stopStart + 1, commits: committedBefore + 1 }, { requests: requestCount('sync'), commits: fixture.committedPages.length });
        await screenshot('stopped-after-current-page');
        stage('sync survives route unmount, stops, and resumes');
        fixture.connectionId = 'current-gmail-generation-4';
        const unmountStart = requestCount('sync');
        fixture.syncPlan.push({ threads: 3, caughtUp: false, hold: 'unmounted-page-one' }, { threads: 4, caughtUp: false, hold: 'unmounted-page-two' });
        await header(syncLabel).click(); await held('unmounted-page-one');
        await page.evaluate(() => { location.hash = '/tasks'; });
        await email.waitFor({ state: 'hidden' });
        await release('unmounted-page-one'); await held('unmounted-page-two');
        check(requestCount('sync') === unmountStart + 2 && !await email.isVisible(), 'app-owned sync dispatches the next page while connected Email is unmounted');
        await page.evaluate(() => { location.hash = '/inbox'; });
        await email.waitFor(); await readyRows();
        check(await header('Syncing…').isDisabled() && await header('Stop sync').isVisible(), 'returning to Email reflects the still-running app-owned job');
        await header('Stop sync').click();
        await release('unmounted-page-two'); await syncIdle();
        check(requestCount('sync') === unmountStart + 2, 'return-and-Stop does not dispatch a third page');
        await screenshot('route-return-stopped');
        fixture.connectionId = 'current-gmail-generation-5';
        fixture.syncPlan.push({ threads: 5, caughtUp: true });
        await header(syncLabel).click(); await syncIdle();
        const resumed = state.requests.filter(request => request.type === 'sync').at(-1);
        check(resumed.body.mailbox_id === fixture.primary && resumed.body.restart === false && resumed.body.connection_id === fixture.connectionId && await email.locator('[data-mail-notice]').innerText() === `${providerName} sync finished. Threads updated: 5.`, 'resume uses the saved mailbox and fresh authorization without restarting its cursor', { mailbox: fixture.primary, restart: false, connection: fixture.connectionId }, resumed.body);
        fixture.syncPlan.push({ threads: 1, caughtUp: true });
        await reachable(email.getByRole('button', { name: 'Restart sync', exact: true }), 'Restart sync action');
        await email.getByRole('button', { name: 'Restart sync', exact: true }).click(); await syncIdle();
        check(state.requests.filter(request => request.type === 'sync').at(-1).body.restart === true, 'explicit Restart sync is the only tested action that requests restart');
        await layout('sync completion and notices');
        await screenshot('resume-finished');
        check((await readImported(page)).length === 0, 'connected server-cache sync does not silently import provider records into device-only Inbox');
      } else if (kind === 'stale-host-responses') {
        await openEmail();
        stage('stale query response is fenced');
        fixture.hold('browse', 'old-query', record => record.query.query === queryNeedle);
        await search().fill(queryNeedle); await search().press('Enter'); await held('old-query');
        await applySearch('NEW_QUERY_HAS_NO_MATCH_Ω');
        check(await rows().count() === 0, 'newer query reaches its empty result before the old request returns');
        await release('old-query');
        check(await rows().count() === 0 && await search().inputValue() === 'NEW_QUERY_HAS_NO_MATCH_Ω', 'late older search result cannot replace the newer query view');
        await applySearch('');
        stage('stale account browse and closed detail are fenced');
        fixture.hold('browse', 'old-account', record => record.pathname.includes(encodeURIComponent(fixture.primary)));
        await header('Refresh').click(); await held('old-account');
        await accounts().nth(1).click(); await readyRows();
        await release('old-account');
        check(await accounts().nth(1).getAttribute('aria-pressed') === 'true' && await rows().count() === 0, 'late prior-account rows cannot replace the selected empty account');
        await accounts().nth(0).click(); await readyRows();
        fixture.hold('thread', 'closed-reader');
        await rows().first().click(); await held('closed-reader');
        await email.getByRole('button', { name: 'Back to email', exact: true }).click();
        await release('closed-reader');
        check(!await email.locator('[data-mail-reader]').isVisible() && await rows().count() > 0, 'a late thread response cannot reopen a reader after Back');
        stage('refresh fences old-host browse and reader responses');
        fixture.hold('browse', 'old-host-rows');
        fixture.hold('thread', 'old-host-thread');
        await header('Refresh').click(); await held('old-host-rows');
        await rows().first().click(); await held('old-host-thread');
        const oldHost = fixture.host;
        fixture.host = 'qa host/B 世界 &?'; fixture.primary = 'new/mailbox 世界 &?'; fixture.secondary = 'new/empty 世界'; fixture.prefix = 'HOST_B'; fixture.accountLabel = 'Different server owner 世界';
        await header('Refresh').click(); await readyRows();
        await until(async () => (await rows().first().innerText()).includes('HOST_B subject'), 'new host rows render');
        await release('old-host-rows'); await release('old-host-thread');
        check(!await email.locator('[data-mail-reader]').isVisible() && (await rows().first().innerText()).includes('HOST_B subject') && !(await email.innerText()).includes('HOST_A subject'), 'refresh server identity change rejects both stale prior-host rows and detail', { newHost: fixture.host, oldHost }, await rows().first().innerText());
        await screenshot('host-changed-stale-responses-released');
        stage('sync request host is fenced when server identity changes during connection read');
        fixture.hold('connection', 'connection-before-host-change');
        await header(syncLabel).click(); await held('connection-before-host-change');
        const syncHost = fixture.host;
        fixture.host = 'qa host/C 林'; fixture.primary = 'third/mailbox 林'; fixture.secondary = 'third/empty 林'; fixture.prefix = 'HOST_C'; fixture.accountLabel = 'Third server owner 林';
        await header('Refresh').click(); await readyRows();
        await release('connection-before-host-change'); await syncIdle();
        const fenced = state.requests.filter(request => request.type === 'sync').at(-1);
        check(fenced.body.host_id === syncHost && fixture.committedPages.length === 0 && await email.locator('[data-mail-sync-error]').innerText() === 'Synthetic server identity changed. Refresh saved mail.', 'old-host sync is rejected by the explicit server-identity contract and remains a visible error', { rejectedHost: syncHost, activeHost: fixture.host, commits: 0 }, { body: fenced.body, commits: fixture.committedPages.length });
        check((await rows().first().innerText()).includes('HOST_C subject'), 'host-fenced sync error refresh preserves the current host cache view');
        await layout('new-host cache after stale responses');
        await screenshot('host-fenced-sync-error');
      } else if (kind.startsWith('attachments-')) {
        await openEmail();
        stage('explicit attachment controls and lazy binary access');
        await rows().first().click();
        await email.locator('[data-mail-message-body]').waitFor();
        await until(async () => await downloadButtons().count() === attachmentFixtures.length, 'attachment controls render');
        const originalBody = await email.locator('[data-mail-message-body]').textContent();
        check(requestCount('attachment') === 0 && state.downloads.length === 0, 'opening a cached thread with inline/empty/large/HTML attachments does not read attachment bytes');
        await reachable(downloadButtons().nth(0), 'Download attachment action');
        await reachable(downloadButtons().nth(1), 'Zero-byte inline download action');
        await layout('attachment metadata and explicit download controls');
        await screenshot('attachment-controls');
        if (kind === 'attachments-download-success') {
          stage('real browser files preserve binary, empty, large and HTML bytes');
          const timerBefore = (await blobSnapshot('before-success-downloads')).timers.length;
          for (let index = 0; index < 4; index++) await downloadSuccess(index, 'success-' + index);
          const timers = (await blobSnapshot('after-success-downloads')).timers.slice(timerBefore);
          check(timers.includes(20_000) && timers.includes(180_000), 'download registers native20s current-connection and180s binary-fetch deadlines', [20_000, 180_000], timers);
          check(await email.locator('[data-mail-message-body]').textContent() === originalBody, 'downloaded HTML/binary bytes do not replace the saved plain-text reader');
          await screenshot('four-downloads-started');
          stage('64MiB ceiling rejects oversized saved metadata before binary access');
          const reads = requestCount('attachment'), files = state.downloads.length, urls = (await blobSnapshot('before-size-guard')).created.length;
          await scrollReaderToAction(4);
          await downloadButtons().nth(4).click();
          await email.locator('[data-mail-download-error]').waitFor();
          await until(() => downloadButtons().nth(4).isEnabled(), 'oversize request stops being busy');
          const after = await blobSnapshot('after-size-guard');
          check(requestCount('attachment') === reads && state.downloads.length === files && after.created.length === urls, '64MiB+1metadata starts no binary request, Blob URL, or file download', { reads, files, urls }, { reads: requestCount('attachment'), files: state.downloads.length, urls: after.created.length });
          check((await email.locator('[data-mail-download-error]').innerText()).includes('No file download started.') && !await email.locator('[data-mail-notice]').isVisible(), 'size-guard failure shows recovery without a stale successful-download notice');
          await feedbackViewport(email.locator('[data-mail-download-error]'), 'Size-guard error feedback');
          await screenshot('oversize-guard');
        } else if (kind === 'attachments-outlook-capabilities') {
          stage('Outlook MIME export receipt differs from stored item size');
          const index = fixture.attachmentSpecs.findIndex(spec => spec.exportSize != null);
          assert(index >= 0, 'Explicit Outlook item fixture required');
          await downloadSuccess(index, 'outlook-item-export');
          check(fixture.attachmentSpecs[index].size !== fixture.attachmentSpecs[index].bytes.length, 'Outlook item export fixture exercises a different stored and downloaded size');
          stage('malformed and oversized export receipts start no download');
          for (const invalid of ['bad-size', String((64 << 20) + 1)]) {
            const files = state.downloads.length, before = await blobSnapshot('before-invalid-export-' + invalid);
            fixture.attachmentPlan.push({ id: fixture.attachmentSpecs[index].id, exportSizeOverride: invalid });
            await scrollReaderToAction(index); await downloadButtons().nth(index).click();
            await email.locator('[data-mail-download-error]').waitFor();
            await until(() => downloadButtons().nth(index).isEnabled(), 'invalid export receipt is retryable');
            const after = await blobSnapshot('after-invalid-export-' + invalid);
            check(state.downloads.length === files && after.created.length === before.created.length && after.clicks.length === before.clicks.length, 'invalid Outlook export receipt starts no file, Blob URL or anchor', { files, urls: before.created.length }, { files: state.downloads.length, urls: after.created.length });
            check(await email.locator('[data-mail-message-body]').textContent() === originalBody, 'invalid export receipt preserves the saved reader');
          }
          await downloadSuccess(index, 'outlook-export-retry');
          stage('linked attachments are explicit metadata-only capabilities');
          const reads = requestCount('attachment');
          fixture.attachmentSpecs.push({ id: 'linked/reference 林', file_name: 'Linked document', expected_name: 'Linked document', bytes: null, size: 0, reference: true, inline: false });
          await header('Refresh').click(); await readyRows();
          await until(async () => await downloadButtons().count() === fixture.attachmentSpecs.length, 'linked attachment metadata appears');
          check(await downloadButtons().last().isDisabled(), 'Outlook linked attachment cannot start a remote download');
          check((await email.locator('[data-mail-reader]').innerText()).includes('Linked or unsupported attachment. Open it in Outlook.') && requestCount('attachment') === reads, 'linked attachment explains its capability without fetching a remote URL');
          await screenshot('outlook-item-and-linked-capabilities');
        } else if (kind === 'attachments-held-route') {
          stage('busy state prevents duplicates while current connection lookup is held');
          const spec = fixture.attachmentSpecs[0], files = state.downloads.length, connections = requestCount('connection');
          fixture.connectionId = 'download/held generation/林 &?';
          fixture.hold('connection', 'attachment-current-connection');
          fixture.attachmentPlan.push({ id: spec.id, hold: 'attachment-binary-page' });
          await scrollReaderToAction(0);
          await downloadButtons().nth(0).click(); await held('attachment-current-connection');
          check(await downloadButtons().evaluateAll(elements => elements.every(element => element.disabled)), 'one busy download disables every attachment action');
          const firstRect = await downloadButtons().nth(0).boundingBox();
          await page.mouse.click(firstRect.x + firstRect.width / 2, firstRect.y + firstRect.height / 2);
          await downloadButtons().nth(1).scrollIntoViewIfNeeded();
          const secondRect = await downloadButtons().nth(1).boundingBox();
          await page.mouse.click(secondRect.x + secondRect.width / 2, secondRect.y + secondRect.height / 2);
          await settle();
          check(requestCount('connection') === connections + 1 && requestCount('attachment') === 0 && state.downloads.length === files, 'duplicate/sibling clicks cannot dispatch a second connection or binary read while busy', { connections: connections + 1, reads: 0, files }, { connections: requestCount('connection'), reads: requestCount('attachment'), files: state.downloads.length });
          await release('attachment-current-connection'); await held('attachment-binary-page');
          check(await email.getByText('Downloading the attachment…', { exact: true }).isVisible(), 'held binary transfer has an explicit pending status');
          await feedbackViewport(email.getByText('Downloading the attachment…', { exact: true }), 'Pending download feedback');
          await screenshot('held-binary-busy');
          stage('held attachment survives real route unmount and return');
          await page.evaluate(() => { location.hash = '/tasks'; });
          await email.waitFor({ state: 'hidden' });
          await page.evaluate(() => { location.hash = '/inbox'; });
          await email.waitFor(); await readyRows();
          await email.locator('[data-mail-message-body]').waitFor();
          await scrollReaderToAction(0);
          check(await downloadButtons().evaluateAll(elements => elements.every(element => element.disabled)) && await email.getByText('Downloading the attachment…', { exact: true }).isVisible(), 'app-owned pending download remains busy after Email unmount/remount');
          await feedbackViewport(email.getByText('Downloading the attachment…', { exact: true }), 'Remounted pending download feedback');
          await screenshot('route-return-still-busy');
          await page.evaluate(() => { location.hash = '/tasks'; });
          await email.waitFor({ state: 'hidden' });
          await release('attachment-binary-page');
          await saveProof(files, spec);
          check(!await email.isVisible() && page.url().endsWith('#/tasks'), 'explicit attachment download completes while Email remains unmounted');
          await page.evaluate(() => { location.hash = '/inbox'; });
          await email.waitFor(); await readyRows();
          await email.locator('[data-mail-message-body]').waitFor();
          await scrollReaderToAction(0);
          check(await downloadButtons().nth(0).isEnabled() && requestCount('attachment') === 1 && state.downloads.length === files + 1 && await email.locator('[data-mail-notice]').innerText() === 'Attachment download started. Check your browser\'s downloads for completion.', 'return shows the completed start status, clears busy state and does not repeat the binary read');
          await feedbackViewport(email.locator('[data-mail-notice]'), 'Remounted download success feedback');
          check(await email.locator('[data-mail-message-body]').textContent() === originalBody, 'route-surviving download retains the saved thread content');
          await screenshot('route-return-download-started');
        } else {
          stage('disconnected current authorization cannot read attachment bytes');
          fixture.connectionState = 'needs_sign_in';
          await scrollReaderToAction(0);
          await downloadButtons().nth(0).click();
          await email.locator('[data-mail-download-error]').waitFor();
          check(await email.locator('[data-mail-download-error]').innerText() === `Connect ${providerName} before downloading this attachment.` && requestCount('attachment') === 0 && state.downloads.length === 0, 'disconnected current generation never falls back to cached attachment authorization');
          await feedbackViewport(email.locator('[data-mail-download-error]'), 'Current-connection error feedback');
          fixture.connectionState = 'connected';
          await downloadSuccess(0, 'after-reconnect');
          stage('long bounded status region scrolls independently of the reader');
          const longMessage = 'Synthetic long recovery token ' + 'x'.repeat(440) + ' END_RECOVERY_RETRY';
          const longFiles = state.downloads.length;
          fixture.syncPlan.push({ status: 507, error: 'Synthetic long sync context ' + 'y'.repeat(440) + ' END_SYNC_RECOVERY' });
          await header(syncLabel).click(); await syncIdle();
          check((await email.locator('[data-mail-sync-error]').innerText()).includes('END_SYNC_RECOVERY'), 'long combined feedback retains the independent sync error after its cache refresh');
          fixture.attachmentPlan.push({ id: fixture.attachmentSpecs[0].id, status: 403, message: longMessage });
          await scrollReaderToAction(0);
          await downloadButtons().nth(0).click();
          await email.locator('[data-mail-download-error]').waitFor();
          const region = email.locator('[data-mail-feedback]');
          const beforeScroll = await region.evaluate(element => {
            const box = element.getBoundingClientRect(), hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
            return { height: box.height, y: box.y, width: box.width, scrollHeight: element.scrollHeight, clientHeight: element.clientHeight, scrollTop: element.scrollTop, maxHeight: getComputedStyle(element).maxHeight, overflowY: getComputedStyle(element).overflowY, tabindex: element.tabIndex, hit: Boolean(hit && (hit === element || element.contains(hit))), viewportHeight: innerHeight };
          });
          check(beforeScroll.height <= 128.1 && beforeScroll.maxHeight === '128px' && beforeScroll.scrollHeight > beforeScroll.clientHeight && beforeScroll.y >= 0 && beforeScroll.y + beforeScroll.height <= beforeScroll.viewportHeight && beforeScroll.hit && beforeScroll.tabindex === 0, 'long download error remains in a bounded viewport-visible, hit-testable, independently scrollable status region', { maxHeight: 128, independentlyScrollable: true }, beforeScroll);
          await region.focus(); await region.press('End');
          await until(() => region.evaluate(element => element.scrollTop >= element.scrollHeight - element.clientHeight - 1), 'keyboard End reaches the end of the long feedback region');
          const endGeometry = await region.evaluate(element => {
            const error = element.querySelector('[data-mail-download-error]'), walker = document.createTreeWalker(error, NodeFilter.SHOW_TEXT), range = document.createRange();
            let text;
            while (walker.nextNode()) if (walker.currentNode.textContent.includes('END_RECOVERY_RETRY')) text = walker.currentNode;
            if (!text) throw new Error('Long feedback final marker is absent');
            const start = text.textContent.indexOf('END_RECOVERY_RETRY');
            range.setStart(text, start); range.setEnd(text, start + 'END_RECOVERY_RETRY'.length);
            const end = range.getBoundingClientRect(), box = element.getBoundingClientRect();
            return { regionScroll: element.scrollTop, lastLineTop: end.top, lastLineBottom: end.bottom, regionTop: box.top, regionBottom: box.bottom, active: document.activeElement === element };
          });
          check(endGeometry.active && endGeometry.regionScroll > 0 && endGeometry.lastLineTop >= endGeometry.regionTop && endGeometry.lastLineBottom <= endGeometry.regionBottom + 1 && state.downloads.length === longFiles, 'keyboard scrolling exposes the complete long recovery text without starting a download', { finalMarkerVisible: true, files: longFiles }, endGeometry);
          check(await email.locator('[data-mail-message-body]').textContent() === originalBody, 'long status scrolling preserves the cached reader');
          await screenshot('long-bounded-feedback-keyboard-end');
          fixture.syncPlan.push({ threads: 0, caughtUp: true });
          await header(syncLabel).click(); await syncIdle();
          check(!await email.locator('[data-mail-sync-error]').count(), 'explicit successful sync clears the long independent sync error before attachment retry');
          await downloadSuccess(0, 'retry-long-feedback');
          const failures = [
            { label: 'offline', network: true, message: 'Could not reach the server:' },
            { label: 'forbidden403', status: 403, message: `Synthetic attachment permission denied. Reconnect ${providerName}.` },
            { label: 'host409', status: 409, message: 'Synthetic attachment host/account changed. Refresh saved mail.' },
            { label: 'rate429', status: 429, message: 'Synthetic provider rate limited. Retry download.' },
            { label: 'truncated', truncated: true, message: 'The attachment could not be read completely. No file download started. Refresh the thread and retry.' },
          ];
          for (const failure of failures) {
            stage('attachment ' + failure.label + ' fails without losing cache and retries');
            const files = state.downloads.length, before = await blobSnapshot('before-failure-' + failure.label);
            fixture.connectionId = 'download/failure generation/' + failure.label + '/林 &?';
            fixture.attachmentPlan.push({ id: fixture.attachmentSpecs[0].id, ...failure });
            await scrollReaderToAction(0);
            await downloadButtons().nth(0).click();
            await email.locator('[data-mail-download-error]').waitFor();
            await until(() => downloadButtons().nth(0).isEnabled(), 'failed attachment becomes retryable');
            const error = await email.locator('[data-mail-download-error]').innerText(), after = await blobSnapshot('after-failure-' + failure.label);
            check(error.includes(failure.message), failure.label + ' displays the actual recovery error', failure.message, error);
            check(state.downloads.length === files && after.created.length === before.created.length && after.clicks.length === before.clicks.length, failure.label + ' starts no browser file, Blob URL or download anchor', { files, urls: before.created.length, clicks: before.clicks.length }, { files: state.downloads.length, urls: after.created.length, clicks: after.clicks.length });
            check(!await email.locator('[data-mail-notice]').isVisible(), failure.label + ' clears the prior successful-download notice instead of claiming success');
            check(await email.locator('[data-mail-message-body]').textContent() === originalBody, failure.label + ' leaves the actual cached reader intact');
            await feedbackViewport(email.locator('[data-mail-download-error]'), failure.label + ' current download error feedback');
            await screenshot('download-' + failure.label + '-error');
            await header('Refresh').click(); await readyRows();
            await email.locator('[data-mail-message-body]').waitFor();
            check(await email.locator('[data-mail-message-body]').textContent() === originalBody && (await email.locator('[data-mail-download-error]').innerText()).includes(failure.message), failure.label + ' recovery refresh retains both cached content and the download error');
            await downloadSuccess(0, 'retry-' + failure.label);
          }
          await screenshot('all-error-retries-recovered');
        }
        await layout('attachment final state');
        check((await readImported(page)).length === 0, 'attachment actions never replace or populate device-only imported Inbox records');
      } else {
        stage('deliberate network-offline read and retry');
        fixture.emptyAccounts = true;
        fixture.failNext = { kind: 'snapshot', network: true };
        await openEmail();
        await email.getByRole('alert').filter({ hasText: 'Could not reach the server:' }).waitFor();
        check(await header(syncLabel).isDisabled(), 'initial offline account read leaves sync disabled instead of inventing an account');
        await screenshot('offline-read-error');
        await header('Refresh').click(); await readyRows();
        check(!await email.getByRole('alert').count() && await accounts().count() === 0 && await email.locator('[data-mail-empty]').innerText() === 'Connect Gmail or Outlook, then sync to save mail on this server.', 'explicit Refresh recovers offline read into a truthful empty-account prompt');
        fixture.connectionState = 'disconnected';
        const beforeSync = requestCount('sync'), snapshots = requestCount('snapshot');
        await header(syncLabel).click(); await syncIdle();
        check(await email.locator('[data-mail-sync-error]').innerText() === `Connect ${providerName} in Connections, then retry sync.` && requestCount('sync') === beforeSync && requestCount('snapshot') > snapshots, 'disconnected current mail generation never submits a sync and its recovery survives refresh');
        await reachable(header('Connections'), 'Empty-account Connections action');
        await layout('offline retry and empty-account recovery');
        await screenshot('empty-accounts-connection-recovery');
      }
    } catch (error) {
      check(false, 'scenario completes without an unexpected harness/UI interaction failure', 'complete intended flow', { error: error.stack ?? String(error) });
      await screenshot('failure').catch(() => {});
    } finally {
      stage('runtime and isolation');
      await Promise.allSettled(downloadJobs);
      try {
        const snapshot = await blobSnapshot('case-final');
        check(snapshot.active.length === 0 && snapshot.remainingAnchors === 0, 'case leaves no active attachment Blob URL or temporary download anchor', { active: 0, anchors: 0 }, { active: snapshot.active, anchors: snapshot.remainingAnchors });
        check(snapshot.payloadExecuted === false, 'attachment payload remains unexecuted at case completion');
      } catch (error) { check(false, 'native Blob/anchor observer remains readable through the case', 'available observer', String(error)); }
      state.heldResponses = fixture.holds.map(item => ({ label: item.label, request: item.record, released: item.released, response: item.value }));
      state.committedSyntheticSyncPages = clone(fixture.committedPages);
      // Chromium can report a navigation-to-download transition as ERR_ABORTED.
      // Only an app-owned Blob URL with an actually completed saved file can
      // qualify; API failures and failed downloads cannot use this classifier.
      const completedBlobDownload = url => state.downloads.some(download => download.url === url && url.startsWith('blob:') && !download.error && download.failure === null);
      // The two explicit invalid export-size responses are refused before their
      // bodies are read. Chromium may report that body cancellation as aborted.
      // Bind it to the exact fixture request, never another API read or retry.
      const rejectedExportBody = error => mailProvider === 'outlook' && kind === 'attachments-outlook-capabilities' && error.error === 'net::ERR_ABORTED' && state.rejectedExportHeaders.some(expected => expected.sequence === error.requestSequence && expected.url === error.url);
      const expectedConsole = error => state.expectedErrors.some(expected => error.url === expected.url && (expected.status ? error.text.includes('status of ' + expected.status) : error.text.includes(expected.network))) || (completedBlobDownload(error.url) && error.text.includes('net::ERR_ABORTED'));
      const unexpectedConsole = state.consoleErrors.filter(error => !expectedConsole(error));
      const verifiedReadCancellation = error => kind === 'stale-host-responses' && error.error === 'net::ERR_ABORTED' && state.readCancellations.some(expected => expected.sequence === error.requestSequence && expected.url === error.url && expected.verifiedBeforeRelease && expected.nativeRequest.signalPresent && expected.nativeRequest.aborted === true && expected.released && expected.browserResponsesAfterRelease.length === 0);
      const unexpectedFailures = state.requestFailures.filter(error => !state.expectedErrors.some(expected => expected.network && expected.url === error.url && error.error?.includes(expected.network)) && !(completedBlobDownload(error.url) && error.error === 'net::ERR_ABORTED') && !rejectedExportBody(error) && !verifiedReadCancellation(error));
      state.verifiedReadCancellationFailures = state.requestFailures.filter(verifiedReadCancellation);
      if (kind === 'stale-host-responses') {
        check(state.readCancellations.length === 5 && state.verifiedReadCancellationFailures.length === 5 && new Set(state.verifiedReadCancellationFailures.map(error => error.requestSequence)).size === 5, 'only five explicitly superseded held GETs classify native cancellations exactly once', { verifiedReads: 5, canceledRequests: 5, uniqueRequests: 5 }, { verifiedReads: state.readCancellations.length, canceledRequests: state.verifiedReadCancellationFailures.length, uniqueRequests: new Set(state.verifiedReadCancellationFailures.map(error => error.requestSequence)).size });
      }
      state.downloadNavigationTransitions = state.requestFailures.filter(error => completedBlobDownload(error.url) && error.error === 'net::ERR_ABORTED');
      state.rejectedExportBodyCancellations = state.requestFailures.filter(rejectedExportBody);
      if (kind === 'attachments-outlook-capabilities') {
        check(state.rejectedExportHeaders.length === 2 && new Set(state.rejectedExportBodyCancellations.map(error => error.requestSequence)).size === state.rejectedExportBodyCancellations.length, 'only the two explicitly invalid export-header fixture requests can classify body cancellations, each at most once', { invalidResponses: 2, uniqueCanceledRequests: true }, { invalidResponses: state.rejectedExportHeaders.length, cancellations: state.rejectedExportBodyCancellations });
      }
      state.consoleClassification = { expected: state.consoleErrors.filter(expectedConsole), unexpected: unexpectedConsole };
      check(state.runtimeErrors.length === 0, 'no JavaScript/WASM runtime error', [], state.runtimeErrors);
      check(unexpectedConsole.length === 0, 'only exact deliberately injected HTTP/offline console errors occur', [], unexpectedConsole);
      check(unexpectedFailures.length === 0, 'only exact deliberately injected network failures occur', [], unexpectedFailures);
      check(state.unexpected.length === 0, 'no unexpected API/provider/content request', [], state.unexpected);
      check(state.blockedWrites.length === 0, 'no write outside the explicitly synthetic provider-sync endpoint', [], state.blockedWrites);
      fixture.teardown = true;
      for (const item of fixture.holds) if (!item.released) item.release();
      await context.close();
      await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
      console.log(JSON.stringify({ case: id, passed: report.checks.filter(check => check.case === id && check.passed).length, failed: report.checks.filter(check => check.case === id && !check.passed).length }));
    }
  }
} finally {
  await browser.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
report.finishedAt = new Date().toISOString();
report.summary = { passed: report.checks.filter(check => check.passed).length, failed: report.checks.filter(check => !check.passed).length, cases: report.cells.length, screenshots: report.cells.reduce((count, cell) => count + cell.screenshots.length, 0) };
report.postflight = {
  sources: await Promise.all(sources.map(async source => ({ file: source.file, unchanged: hash(await readFile(path.join(root, source.file))) === source.sha256 }))),
  assets: await Promise.all(assets.map(async asset => ({ file: asset.file, hashUnchanged: hash(await readFile(path.join(dist, asset.file))) === asset.sha256, mtimeUnchanged: String((await stat(path.join(dist, asset.file), { bigint: true })).mtimeNs) === asset.mtime_ns }))),
  fileSetUnchanged: JSON.stringify(await filesIn(dist)) === JSON.stringify(assetNames),
  harnessUnchanged: hash(await readFile(self)) === report.harnessSha256,
};
const integrity = report.postflight;
if (!integrity.harnessUnchanged || !integrity.fileSetUnchanged || integrity.sources.some(source => !source.unchanged) || integrity.assets.some(asset => !asset.hashUnchanged || !asset.mtimeUnchanged)) {
  report.integrityFailure = 'Pinned source/harness/assets changed during the run';
  report.summary.failed++;
}
await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
const artifactNames = (await readdir(output)).filter(name => name !== 'artifact-sha256.json').sort();
await writeFile(path.join(output, 'artifact-sha256.json'), JSON.stringify({ harnessSha256: report.harnessSha256, artifacts: await Promise.all(artifactNames.map(async file => ({ file, sha256: hash(await readFile(path.join(output, file))) }))) }, null, 2) + '\n');
console.log(JSON.stringify({ ...report.summary, artifacts: output, scope: report.scope }));
if (report.summary.failed) process.exitCode = 1;
