// Opt-in real-provider and macOS Keychain smoke test. Uses only generated canaries.
// Build for http://127.0.0.1:43013 and pass workspace root, isolated dist, --live.
// BRUHS_PLAYWRIGHT_PATH and BRUHS_LIVE_CODEX_MODEL name local installations.
import assert from 'node:assert/strict';
import { createHash, randomBytes, randomUUID } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, realpath, writeFile, symlink } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';

assert(process.argv.includes('--live'), 'Pass --live to opt into real provider calls and disposable OS keychain writes');
assert(process.argv[2] && process.argv[3], 'Explicit workspace root and isolated WASM build directory required');
const root = resolve(process.argv[2]);
const dist = resolve(process.argv[3]);
const providers = process.argv.includes('--claude-only') ? ['claude'] : process.argv.includes('--codex-only') ? ['codex'] : ['claude', 'codex'];
if (providers.includes('codex')) assert(process.env.BRUHS_LIVE_CODEX_MODEL, 'Set BRUHS_LIVE_CODEX_MODEL to a model supported by the installed CLI/account');
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const artifacts = await realpath(await mkdtemp(join(tmpdir(), 'bruhs-credentials-live-')));
const testHome = join(artifacts, 'home');
const project = join(testHome, 'credential-smoke');
await mkdir(project, { recursive: true, mode: 0o700 });
await mkdir(join(testHome, 'Library/Preferences'), { recursive: true, mode: 0o700 });
// The Security framework finds the user's login keychain through these paths.
// Link only keychain selection metadata; the app catalog remains disposable.
await symlink(join(homedir(), 'Library/Keychains'), join(testHome, 'Library/Keychains'));
await symlink(join(homedir(), 'Library/Preferences/com.apple.security.plist'), join(testHome, 'Library/Preferences/com.apple.security.plist'));
const origin = 'http://127.0.0.1:43013';
const originals = providers.map(() => randomBytes(32).toString('hex'));
const replacements = providers.map(() => randomBytes(32).toString('hex'));
const privateValues = [...originals, ...replacements];
const names = providers.map(p => `Bruhs live smoke ${p} ${randomUUID()}`);
const ids = names.map(name => createHash('sha256').update(project).update('\0').update(name).digest('hex'));
const sha = value => createHash('sha256').update(value).digest('hex');
const sanitize = value => privateValues.reduce((text, secret) => text.replaceAll(secret, '[private canary]'), String(value));
const report = { artifacts, origin, project, providers, checks: [], turns: [], pageErrors: [], leaks: [], cleanup: {}, transcriptAudits: [], canaryHashes: privateValues.map(sha), scope: 'Real installed Claude/Codex, actual app WASM/API, actual macOS Keychain, disposable app HOME/database/project/browser. No user credential values are read.' };
report.testAccounts = providers.map((provider, i) => ({ provider, id: ids[i], name: names[i], project }));
await writeFile(join(artifacts, 'test-accounts.json'), JSON.stringify(report.testAccounts, null, 2), { mode: 0o600 });
const ownedIds = new Set();
let server, browser, context, page, phase = 'setup', serverLog = '', currentTurn;
let interrupted = false;
const interrupt = signal => { interrupted = true; report.interrupted = signal; process.exitCode = 1; void context?.close().catch(() => {}); };
process.once('SIGINT', interrupt); process.once('SIGTERM', interrupt);
// Restore the original HOME only for provider CLIs so OAuth/keychain identity
// stays unchanged. The app and its credential catalog keep the disposable HOME.
const wrapperDir = join(artifacts, 'bin'); await mkdir(wrapperDir, { mode: 0o700 });
const shellQuote = value => `'${value.replaceAll("'", "'\\''")}'`;
for (const provider of ['claude', 'codex']) {
  const installed = spawnSync('/usr/bin/which', [provider], { encoding: 'utf8' }).stdout.trim();
  assert(installed.startsWith('/'), `Missing ${provider} executable`);
  await writeFile(join(wrapperDir, provider), `#!/bin/sh\nexport HOME=${shellQuote(homedir())}\nexec ${shellQuote(installed)} "$@"\n`, { mode: 0o700 });
}
const env = { ...process.env, HOME: testHome, PATH: `${wrapperDir}:${process.env.PATH}`, BRUHS_SERVER_PORT: '43013', BRUHS_WEB_DIST: dist, BRUHS_MEETING_DB: join(artifacts, 'meetings.sqlite3'), BRUHS_CAPTURE_URL: 'http://127.0.0.1:9', BRUHS_CAPTURE_TOKEN_FILE: join(artifacts, 'no-capture-token'), RUST_LOG: 'warn' };
for (const name of ['BRUHS_GOOGLE_CLIENT_ID', 'BRUHS_GOOGLE_CLIENT_SECRET', 'BRUHS_API_URL', 'BRUHS_WEB_ORIGIN', 'ANTHROPIC_API_KEY', 'BRUHS_CLAUDE_USE_API_KEY']) delete env[name];
const python = spawnSync('python3', ['-c', 'import sys; print(sys.executable)'], { encoding: 'utf8' }).stdout.trim();
assert(python.startsWith('/'), 'Python interpreter must be absolute');
assert.equal(process.platform, 'darwin', 'This live harness currently verifies macOS Keychain');
const keychainStatus = id => spawnSync('/usr/bin/security', ['find-generic-password', '-s', 'bruhs-ai-secrets', '-a', id], { encoding: 'utf8' }).status;
const waitFor = async (test, description, timeout = 120000) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { assert(!interrupted, 'Live test interrupted'); if (await test()) return; await new Promise(r => setTimeout(r, 250)); }
  throw new Error(`Timed out: ${description}`);
};
const api = async (path, options = {}) => {
  const response = await fetch(origin + path, { ...options, headers: { Origin: origin, ...(options.body ? { 'Content-Type': 'application/json' } : {}), ...options.headers }, signal: AbortSignal.timeout(15000) });
  const text = await response.text();
  assert(!privateValues.some(value => text.includes(value)), 'Credential leaked into an API response');
  return { status: response.status, body: text ? JSON.parse(text) : undefined };
};
const card = () => page.getByRole('region', { name: 'Private credential request', exact: true });
const stop = () => page.getByRole('button', { name: /^(Stop generation|Interrupt)$/ });
const entry = name => page.locator('#credentials').getByRole('article', { name: `Saved credential: ${name}`, exact: true });
async function walk(directory) {
  const files = [];
  for (const item of await readdir(directory, { withFileTypes: true }).catch(error => { if (error.code === 'ENOENT') return []; throw error; })) {
    const path = join(directory, item.name);
    if (item.isDirectory()) files.push(...await walk(path));
    else if (item.isFile()) files.push(path);
  }
  return files;
}
async function auditTurnLogs(turn) {
  const session = turn.events.find(event => event.type === 'init')?.session_id;
  assert(session && /^[a-zA-Z0-9-]+$/.test(session), 'Missing safe provider session ID');
  const directory = turn.provider === 'claude' ? (process.env.CLAUDE_CONFIG_DIR || join(homedir(), '.claude')) : (process.env.CODEX_HOME || join(homedir(), '.codex'));
  const files = (await walk(join(directory, turn.provider === 'claude' ? 'projects' : 'sessions'))).filter(path => path.includes(session));
  assert(files.length > 0, 'No transcript found for the smoke test session');
  if (turn.provider === 'claude') files.push(join(directory, 'debug', `${session}.txt`));
  let checked = 0;
  for (const file of files) {
    const text = await readFile(file, 'utf8').catch(error => { if (error.code === 'ENOENT') return undefined; throw error; });
    if (text === undefined) continue;
    assert(!privateValues.some(value => text.includes(value)), 'Private canary entered a provider transcript/log');
    checked++;
  }
  report.transcriptAudits.push({ phase: turn.phase, provider: turn.provider, session, filesChecked: checked, credentialAbsent: true });
}
async function privacy() {
  const stored = await page.evaluate(async () => {
    const result = { local: { ...localStorage }, session: { ...sessionStorage }, databases: {} };
    for (const { name } of await indexedDB.databases()) {
      const db = await new Promise((resolve, reject) => { const q = indexedDB.open(name); q.onsuccess = () => resolve(q.result); q.onerror = () => reject(q.error); });
      try {
        result.databases[name] = {};
        for (const store of db.objectStoreNames) result.databases[name][store] = await new Promise((resolve, reject) => { const q = db.transaction(store).objectStore(store).getAll(); q.onsuccess = () => resolve(q.result); q.onerror = () => reject(q.error); });
      } finally { db.close(); }
    }
    return result;
  });
  for (const text of [JSON.stringify(stored), await page.locator('body').innerText(), serverLog]) assert(!privateValues.some(value => text.includes(value)), 'Canary appeared outside a private credential field/request');
  const catalog = await readFile(join(testHome, 'Library/Application Support/dev.bruhs.bruhs-ai/credentials/catalog.json'), 'utf8').catch(error => { if (error.code === 'ENOENT') return ''; throw error; });
  assert(!privateValues.some(value => catalog.includes(value)), 'Catalog contains a credential value');
  assert.equal(report.leaks.length, 0, 'Private value crossed the credential transport boundary');
  for (const file of await walk(testHome)) {
    const bytes = await readFile(file);
    assert(!privateValues.some(value => bytes.includes(Buffer.from(value))), 'Private canary entered an app data file');
  }
}
async function openSettings() {
  await page.goto(origin + '/#/settings');
  await page.locator('#credentials').getByRole('heading', { name: 'Credentials', exact: true }).waitFor({ timeout: 45000 });
  await page.getByRole('navigation', { name: 'Settings sections', exact: true }).getByRole('button', { name: 'Credentials', exact: true }).click();
}
async function start(provider, name, expected, label) {
  phase = `${provider}-${label}`;
  console.error(JSON.stringify({ phase, step: 'open-thread' }));
  const thread = randomUUID();
  currentTurn = { phase, thread, provider, events: [] };
  report.turns.push(currentTurn);
  await page.goto(origin + '/#/chat');
  await page.locator('#chat-input').waitFor({ timeout: 45000 });
  await page.waitForFunction(() => !document.querySelector('[aria-label="Thread list loading status"]') && !document.querySelector('[aria-label="Conversation loading status"]'));
  const model = provider === 'claude' ? (process.env.BRUHS_LIVE_CLAUDE_MODEL || 'claude-sonnet-4-6') : process.env.BRUHS_LIVE_CODEX_MODEL;
  await page.evaluate(async ({ thread, project, model, title }) => {
    localStorage.setItem('bruhs.model', model); localStorage.setItem('bruhs.permissionMode', 'default');
    const db = await new Promise((resolve, reject) => { const q = indexedDB.open('bruhs'); q.onsuccess = () => resolve(q.result); q.onerror = () => reject(q.error); });
    try {
      await new Promise((resolve, reject) => {
        const tx = db.transaction(['threads'], 'readwrite');
        tx.objectStore('threads').put({ id: thread, title, project_name: '', project_path: project, model, updated_at: String(Date.now()), pinned: false, settled: false, run_locked: true });
        tx.oncomplete = resolve; tx.onabort = () => reject(tx.error);
      });
    } finally { db.close(); }
  }, { thread, project, model, title: phase });
  await page.reload();
  await page.locator('#chat-input').waitFor({ timeout: 45000 });
  await page.waitForFunction(() => !document.querySelector('[aria-label="Thread list loading status"]') && !document.querySelector('[aria-label="Conversation loading status"]'));
  await page.locator(`button[data-thread-id="${thread}"]`).click();
  console.error(JSON.stringify({ phase, step: 'submit-turn' }));
  const command = [python, '-c', `import hashlib,os,sys; value=os.environ['SMOKE_CREDENTIAL'].encode(); os.write(1,value); os.write(2,value); sys.exit(0 if hashlib.sha256(value).hexdigest() == '${sha(expected)}' else 41)`];
  const operation = { name, reason: 'Verify an isolated credential canary is injected into this exact digest-check command.', env_var: 'SMOKE_CREDENTIAL', command };
  const prompt = `This is an isolated private credential harness smoke test. Discover and call the bruhs_secrets request_secret MCP tool exactly once using these exact arguments: ${JSON.stringify(operation)}. You may use built-in tool search or functions.exec with ALL_TOOLS to discover and invoke that tool. Do not read files or environment, use shell/web tools, delegate, or ask for credentials in chat. The tool executes the reviewed command; do not run it yourself. Wait for its result and then report the status and exit code briefly. If unavailable, report the error and stop.`;
  await page.locator('#chat-input').fill(prompt);
  const submitted = page.waitForResponse(response => response.url() === origin + '/api/agent' && response.status() === 200, { timeout: 30000 });
  await page.locator('#chat-input').press('Enter'); await submitted;
  console.error(JSON.stringify({ phase, step: 'wait-private-prompt' }));
  await waitFor(async () => {
    if (await card().isVisible()) return true;
    const failure = currentTurn.events.find(event => event.type === 'error' || (event.type === 'result' && event.is_error));
    if (failure) throw new Error(`Provider failed before private prompt: ${sanitize(JSON.stringify(failure))}`);
    if (provider === 'codex' && currentTurn.events.some(event => event.type === 'usage')) {
      const text = currentTurn.events.filter(event => event.type === 'text').map(event => event.text).join('\n');
      throw new Error(`Codex completed without opening a private prompt: ${sanitize(text)}`);
    }
    return false;
  }, `${phase} private prompt`);
  await waitFor(() => currentTurn.events.some(event => event.type === 'secret_request'), 'credential SSE recorded', 5000);
  const request = currentTurn.events.find(event => event.type === 'secret_request');
  assert(request, 'Private card has no actual broker event');
  assert.equal(request.thread_id, thread); assert.equal(request.cwd, project);
  assert.deepEqual(request.operation, operation, 'Provider changed the exact reviewed operation');
  assert.equal(await card().getByLabel('Credential', { exact: true }).getAttribute('type'), 'password');
  assert(!request.storage_unavailable, 'Actual OS keychain is unavailable');
  return request;
}
async function finish(expectedStatus = 'completed') {
  await card().waitFor({ state: 'hidden', timeout: 30000 });
  await stop().waitFor({ state: 'hidden', timeout: 120000 });
  await waitFor(() => currentTurn.events.some(event => ['result', 'error', 'usage'].includes(event.type)), `${phase} result`, 5000);
  const result = currentTurn.events.find(event => event.type === 'tool_result' && event.content.includes(`"status":"${expectedStatus}"`));
  assert(result, `Missing ${expectedStatus} operation result; event types: ${currentTurn.events.map(e => e.type).join(',')}`);
  if (expectedStatus === 'completed') assert(result.content.includes('"exit_code":0') && !result.is_error, 'Exact canary digest command failed');
  assert(!currentTurn.events.some(event => event.type === 'permission_request'), 'Private form unexpectedly needed a second generic approval');
  assert(!currentTurn.events.some(event => event.type === 'error' || (event.type === 'result' && event.is_error)), 'Provider turn failed');
  await privacy();
  await auditTurnLogs(currentTurn);
  report.checks.push({ phase, passed: true, operationStatus: expectedStatus, exitCode: expectedStatus === 'completed' ? 0 : null });
  console.error(JSON.stringify(report.checks.at(-1)));
}

try {
  for (const id of ids) { assert.equal(keychainStatus(id), 44, 'A generated test account unexpectedly already exists'); ownedIds.add(id); }
  server = spawn(join(root, 'target/debug/bruhs-server'), [], { env, cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', value => { serverLog += value; }); server.stderr.on('data', value => { serverLog += value; });
  await waitFor(async () => { if (server.exitCode !== null) throw new Error('Isolated server exited'); return fetch(origin + '/api/credentials').then(r => r.ok).catch(() => false); }, 'isolated server', 20000);
  const initial = await api('/api/credentials'); assert.deepEqual(initial.body, [], 'Isolated catalog was not empty');
  for (const provider of providers) {
    const auth = await api(`/api/agent/auth/${provider}`); assert(auth.body.authenticated, `${provider} is not authenticated in isolated app`);
    report.checks.push({ phase: `${provider}-authenticated`, passed: true });
  }
  browser = await chromium.launch({ headless: true });
  context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: 'block', ...(process.env.BRUHS_RECORD_VIDEO ? { recordVideo: { dir: join(artifacts, 'video'), size: { width: 1280, height: 900 } } } : {}) });
  context.setDefaultTimeout(10000);
  await context.exposeBinding('__credentialSmokeEvent', (_source, event) => {
    if (privateValues.some(value => JSON.stringify(event).includes(value))) report.leaks.push({ phase, boundary: 'agent SSE' });
    if (currentTurn) currentTurn.events.push(event);
    if (['secret_request', 'error', 'result'].includes(event.type)) console.error(JSON.stringify({ phase, observed: event.type, isError: event.is_error || false }));
  });
  await context.addInitScript(() => {
    const original = window.fetch;
    window.fetch = async (...args) => {
      const response = await original(...args);
      if (new URL(response.url).pathname === '/api/agent' && response.ok) {
        void (async () => {
          const reader = response.clone().body.getReader(), decoder = new TextDecoder(); let buffer = '';
          while (true) {
            const { value, done } = await reader.read(); if (done) break; buffer += decoder.decode(value, { stream: true });
            let index;
            while ((index = buffer.indexOf('\n\n')) >= 0) {
              const frame = buffer.slice(0, index); buffer = buffer.slice(index + 2);
              for (const line of frame.split('\n')) if (line.startsWith('data:')) void window.__credentialSmokeEvent(JSON.parse(line.slice(5).trimStart()));
            }
          }
        })().catch(() => {});
      }
      return response;
    };
  });
  context.on('request', request => {
    const path = new URL(request.url()).pathname, body = request.postData() || '';
    if (privateValues.some(value => body.includes(value)) && path !== '/api/agent/secret' && !(request.method() === 'PUT' && path.startsWith('/api/credentials/'))) report.leaks.push({ phase, boundary: path });
  });
  page = await context.newPage(); page.on('pageerror', error => report.pageErrors.push({ phase, message: sanitize(error.stack || error) }));
  for (const [i, provider] of providers.entries()) {
    const name = names[i], id = ids[i];
    if (process.argv.includes('--privacy-only')) {
      const request = await start(provider, name, originals[i], 'private-use-once'); assert(!request.saved_available);
      await card().getByLabel('Credential', { exact: true }).fill(originals[i]);
      await privacy();
      await page.screenshot({ path: join(artifacts, `${provider}-masked-prompt-desktop.png`) });
      if (await page.locator('aside[aria-label="Sidebar navigation"]').isVisible()) await page.getByRole('button', { name: 'Toggle sidebar (⌘B)', exact: true }).click();
      await page.locator('aside[aria-label="Sidebar navigation"]').waitFor({ state: 'hidden' });
      await page.setViewportSize({ width: 390, height: 844 });
      await card().getByLabel('Credential', { exact: true }).scrollIntoViewIfNeeded();
      await page.screenshot({ path: join(artifacts, `${provider}-masked-prompt-mobile.png`) });
      await page.setViewportSize({ width: 1280, height: 900 });
      await card().getByRole('button', { name: 'Use once', exact: true }).click(); await finish();
      assert.equal(keychainStatus(id), 44, 'Use-once unexpectedly saved a Keychain item');
      continue;
    }
    const first = await start(provider, name, originals[i], 'save'); assert(!first.saved_available);
    await card().getByLabel('Credential', { exact: true }).fill(originals[i]);
    await privacy();
    await card().getByRole('button', { name: 'Save securely and use', exact: true }).click();
    await finish(); assert.equal(keychainStatus(id), 0, 'Save did not create an actual Keychain item');
    const listed = (await api('/api/credentials')).body;
    assert.deepEqual(listed, [{ id, name, project_path: project, state: 'saved' }], 'Saved metadata did not match the isolated entry');
    await openSettings(); await entry(name).waitFor();
    await page.screenshot({ path: join(artifacts, `${provider}-saved-settings.png`) });
    await entry(name).getByRole('button', { name: 'Replace', exact: true }).click();
    const replacement = entry(name).getByLabel('New credential value', { exact: true }); assert.equal(await replacement.getAttribute('type'), 'password');
    await replacement.fill(replacements[i]); await privacy();
    const replaced = page.waitForResponse(response => response.url() === `${origin}/api/credentials/${id}` && response.request().method() === 'PUT');
    await entry(name).getByRole('button', { name: 'Save replacement', exact: true }).click(); assert.equal((await replaced).status(), 204);
    const reused = await start(provider, name, replacements[i], 'reuse-replacement'); assert(reused.saved_available);
    await card().getByRole('button', { name: 'Use saved', exact: true }).click(); await finish();
    await openSettings(); await entry(name).getByRole('button', { name: 'Delete', exact: true }).click();
    const deleted = page.waitForResponse(response => response.url() === `${origin}/api/credentials/${id}` && response.request().method() === 'DELETE');
    await entry(name).getByRole('button', { name: 'Delete credential', exact: true }).click(); assert.equal((await deleted).status(), 204);
    assert.equal(keychainStatus(id), 44, 'Delete left the actual Keychain item behind'); assert.deepEqual((await api('/api/credentials')).body, []);
    const absent = await start(provider, name, replacements[i], 'deleted-decline'); assert(!absent.saved_available);
    await card().getByRole('button', { name: 'Decline', exact: true }).click(); await finish('declined');
    const cancelled = await start(provider, name, originals[i], 'cancel-pending');
    await card().getByLabel('Credential', { exact: true }).fill(originals[i]);
    await stop().click(); await card().waitFor({ state: 'hidden' }); await stop().waitFor({ state: 'hidden' });
    await waitFor(async () => (await api('/api/agent/secret', { method: 'POST', body: JSON.stringify({ thread_id: cancelled.thread_id, request_id: cancelled.id, decision: { action: 'use_saved' } }) })).status === 404, 'cancelled approval revoked', 5000);
    await privacy(); await auditTurnLogs(currentTurn); report.checks.push({ phase, passed: true, staleSubmissionStatus: 404, keychainUntouched: keychainStatus(id) === 44 }); console.error(JSON.stringify(report.checks.at(-1)));
  }
  assert.equal(report.pageErrors.length, 0, 'Browser runtime errors');
} catch (error) {
  report.error = sanitize(error.stack || error); process.exitCode = 1;
} finally {
  for (const [i, id] of ids.entries()) {
    if (!ownedIds.has(id)) continue;
    try {
      const rows = (await api('/api/credentials')).body;
      const row = rows.find(row => row.id === id);
      if (row) { assert.equal(row.project_path, project); assert.equal(row.name, names[i]); await api(`/api/credentials/${id}`, { method: 'DELETE' }); }
    } catch (error) { report.cleanupApiError = sanitize(error.message); }
    // The API may have crashed. Only accounts proven absent before this test
    // are eligible for the exact-account OS fallback.
    if (keychainStatus(id) === 0) spawnSync('/usr/bin/security', ['delete-generic-password', '-s', 'bruhs-ai-secrets', '-a', id], { encoding: 'utf8' });
    report.cleanup[providers[i]] = keychainStatus(id) === 44;
    if (!report.cleanup[providers[i]]) process.exitCode = 1;
  }
  try { await context?.close(); await browser?.close(); } catch (error) { report.browserCleanupError = sanitize(error.message); process.exitCode = 1; }
  if (server && server.exitCode === null) {
    server.kill('SIGTERM');
    await Promise.race([new Promise(resolve => server.once('exit', resolve)), new Promise(resolve => setTimeout(resolve, 10000))]);
    if (server.exitCode === null) server.kill('SIGKILL');
  }
  process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
  report.serverLog = sanitize(serverLog).slice(-16000);
  report.turns = report.turns.map(({ phase, thread, events }) => ({ phase, thread, eventTypes: events.map(e => e.type), sessionId: events.find(e => e.type === 'init')?.session_id, results: events.filter(e => ['error','result','tool_result','text','tool_call','raw'].includes(e.type)).map(e => ({ type: e.type, is_error: e.is_error, message: sanitize(e.message || e.result || e.content || e.text || JSON.stringify(e.data || e.input) || '').slice(0, 2000) })), requests: events.filter(e => e.type === 'secret_request').map(e => ({ id: e.id, saved_available: e.saved_available, storage_unavailable: e.storage_unavailable })) }));
  const reportPath = join(artifacts, 'report.json'); await writeFile(reportPath, JSON.stringify(report, null, 2), { mode: 0o600 });
  console.error(JSON.stringify({ report: reportPath, checks: report.checks, error: report.error, cleanup: report.cleanup }));
}
