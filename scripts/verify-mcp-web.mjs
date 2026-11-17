// Opt-in end-to-end check of the web MCP workspace connection
// (docs/local-mcp-workspace.md, "Web connection"; PARITY-03 in
// docs/web-desktop-parity-goal-2026-11-15.md). Every write domain is
// proposed through the real bruhs-mcp binary, reviewed, started, run and
// undone in the web app, People and Meetings are read over real records, and
// a stopped chat turn is read back as stopped.
//
//   cargo build -p bruhs-server -p bruhs-mcp
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:39321 trunk build --dist /tmp/bruhs-mcp-web-dist)
//   node scripts/verify-mcp-web.mjs "$PWD" /tmp/bruhs-mcp-web-dist [group,group,...]
//
// The bundle must be built for the port the check uses (BRUHS_VERIFY_PORT,
// default 39321). bruhs-server runs with a disposable HOME, a capture URL
// nothing listens on and, first on its PATH, a stand-in `claude` that streams
// one line and waits to be stopped, so no provider, capture daemon or real
// data is touched. Two contacts and an ended meeting (summary and transcript)
// are seeded through the server's API and store; the rest goes through the
// app. Headless Chrome (BRUHS_VERIFY_CHROME, default the macOS app) runs on
// BRUHS_VERIFY_CDP_PORT (default 9421) with a fresh profile. Results,
// screenshots and logs stay in the printed directory. Unix only, like the
// connection. Groups: the domain keys below plus revoked-midway,
// chat-open-undo, mixed and chat-stop-live; reads always run.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const only = process.argv[4];
const mcpBin = path.join(root, 'target/debug/bruhs-mcp');
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-mcp', mcpBin], ['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
const appPort = Number(process.env.BRUHS_VERIFY_PORT ?? 39321);
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9421);
const chromeBin = process.env.BRUHS_VERIFY_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const runDir = fs.realpathSync(fs.mkdtempSync(path.join(tmpdir(), 'bruhs-mcp-web-')));
const home = path.join(runDir, 'home');
const bin = path.join(runDir, 'bin');
for (const dir of [home, bin, path.join(runDir, 'chrome'), path.join(runDir, 'shots')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
console.log(`Run directory: ${runDir}`);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// A stand-in for the Claude Code CLI: signed in, starts a turn, streams one
// text delta, then waits until the server stops it.
const standIn = path.join(bin, 'claude');
fs.writeFileSync(standIn, `#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai","email":"verify@example.invalid"}'; exit 0 ;;
  --version|-v) echo "2.1.0 (Claude Code)"; exit 0 ;;
esac
printf '%s\\n' '{"type":"system","subtype":"init","session_id":"verify-session-1","model":"claude-verify","tools":[],"mcp_servers":[]}'
printf '%s\\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Starting the long task"}}}'
while :; do sleep 1; done
`, { mode: 0o755 });

const portBusy = async (port) => {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
    return true;
  } catch (error) {
    return error?.cause?.code !== 'ECONNREFUSED';
  }
};
for (const port of [appPort, cdpPort]) {
  if (await portBusy(port)) {
    console.error(`Port ${port} is in use; set BRUHS_VERIFY_PORT / BRUHS_VERIFY_CDP_PORT.`);
    process.exit(2);
  }
}

const children = new Set();
let server = null;
async function startServer() {
  const log = fs.openSync(path.join(runDir, 'server.log'), 'a');
  server = spawn(serverBin, [], {
    env: {
      PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`,
      HOME: home,
      TMPDIR: home,
      BRUHS_SERVER_PORT: String(appPort),
      BRUHS_WEB_DIST: dist,
      BRUHS_CAPTURE_URL: 'http://127.0.0.1:9',
      BRUHS_CAPTURE_TOKEN_FILE: path.join(runDir, 'no-capture-token'),
      RUST_LOG: 'warn',
    },
    stdio: ['ignore', log, log],
  });
  children.add(server);
  for (let attempt = 0; attempt < 150; attempt++) {
    try {
      if ((await fetch(`http://127.0.0.1:${appPort}/api/health`)).ok) return;
    } catch {}
    await sleep(200);
  }
  throw new Error('bruhs-server did not start; see server.log');
}
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(5000).then(() => false)]) === false) child.kill('SIGKILL');
  children.delete(child);
}
async function cleanup() {
  for (const child of [...children].reverse()) await stopChild(child);
  spawnSync('/usr/bin/pkill', ['-f', standIn]);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));

let exitCode = 1;
try {
  await startServer();
  const api = async (method, route, body) => {
    const response = await fetch(`http://127.0.0.1:${appPort}${route}`, { method, headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    if (!response.ok) throw new Error(`${method} ${route}: ${response.status} ${await response.text()}`);
    return response.json().catch(() => null);
  };
  const seededAt = Date.now();
  await api('POST', '/api/people', { id: 'person-ada', display_name: 'Ada Lovelace', created_at: seededAt - 86400000, updated_at: seededAt - 86400000 });
  await api('POST', '/api/people', { id: 'person-grace', display_name: 'Grace Hopper', created_at: seededAt - 86400000, updated_at: seededAt - 86400000 });
  const meetingId = (await api('POST', '/api/meetings', { title: 'Launch sync with Ada', started_at_ms: seededAt - 7200000 })).id;
  await api('PATCH', `/api/meetings/${meetingId}`, { status: 'ended', ended_at_ms: seededAt - 5400000 });
  await api('POST', `/api/meetings/${meetingId}/speakers/them/link/person-ada`);
  // The transcript and summary come from capture and a model in the app;
  // with neither here, write them into the stopped server's store.
  await stopChild(server);
  const find = (dir) => fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    return entry.isDirectory() ? find(full) : entry.name === 'meetings.sqlite3' ? [full] : [];
  });
  const [store] = find(home);
  if (!store) throw new Error('the meetings store was not found under the scratch HOME');
  const db = new DatabaseSync(store);
  const seededMeeting = JSON.parse(db.prepare('SELECT payload FROM meetings WHERE id = ?').get(meetingId).payload);
  seededMeeting.summary = { tldr: 'Ada and I agreed the launch brief ships Friday.', action_items: [], decisions: ['Ship the launch brief on Friday'], topics: [], generated_by: 'seeded-for-verification', generated_at: seededAt - 5000000 };
  db.prepare('UPDATE meetings SET payload = ? WHERE id = ?').run(JSON.stringify(seededMeeting), meetingId);
  const lines = [['me', 0, 4000, 'Thanks for joining, Ada.'], ['them', 4000, 9000, 'Happy to. The brief is nearly done.'], ['me', 9000, 15000, "Great, let's ship it Friday."]];
  lines.forEach(([speaker, start, end, text], index) => {
    const segment = { id: `seg-${index}`, meeting_id: meetingId, speaker_id: speaker, start_ms: start, end_ms: end, text, confidence: 0.9, partial: false, speaker_confidence: null };
    db.prepare('INSERT INTO segments (id, meeting_id, started, payload) VALUES (?, ?, ?, ?)').run(segment.id, meetingId, start, JSON.stringify(segment));
  });
  db.close();
  await startServer();

  const chrome = spawn(chromeBin, ['--headless=new', `--remote-debugging-port=${cdpPort}`, `--user-data-dir=${path.join(runDir, 'chrome')}`, '--no-first-run', '--no-default-browser-check', '--disable-extensions', 'about:blank'], { stdio: 'ignore' });
  children.add(chrome);
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      if ((await fetch(`http://127.0.0.1:${cdpPort}/json/version`)).ok) break;
    } catch {}
    await sleep(200);
  }

  const base = `http://127.0.0.1:${appPort}`;
  const shots = path.join(runDir, 'shots');
  const results = [];
  let group = 'setup';
  const record = (name, pass, detail = '') => {
    results.push({ group, name, pass: !!pass, detail: String(detail).slice(0, 400) });
    console.log(`${pass ? 'PASS' : 'FAIL'} [${group}] ${name}${detail ? ` — ${String(detail).slice(0, 300)}` : ''}`);
    return !!pass;
  };

  // ── CDP ────────────────────────────────────────────────────────────────────
  const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json/list`)).json();
  const target = targets.find((t) => t.type === 'page');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve) => ws.addEventListener('open', resolve, { once: true }));
  let nextId = 1;
  const pending = new Map();
  const listeners = [];
  ws.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    } else if (message.method) {
      for (const listener of listeners) listener(message);
    }
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 500));
    return result.result.value;
  };
  const waitFor = async (expression, timeout = 20000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      try {
        if (await evaluate(expression)) return true;
      } catch {}
      await sleep(150);
    }
    return false;
  };
  const screenshot = async (name) => {
    const { data } = await send('Page.captureScreenshot', { format: 'png' });
    fs.writeFileSync(path.join(shots, name), Buffer.from(data, 'base64'));
  };
  const consoleLines = [];
  const dialogs = [];
  listeners.push((message) => {
    if (message.method === 'Page.javascriptDialogOpening') {
      dialogs.push(`${message.params.type}: ${message.params.message}`);
      send('Page.handleJavaScriptDialog', { accept: true }).catch(() => {});
    }
  });
  listeners.push((message) => {
    if (message.method === 'Runtime.consoleAPICalled') {
      const text = message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ');
      consoleLines.push(`${message.params.type}: ${text}`);
    } else if (message.method === 'Runtime.exceptionThrown') {
      consoleLines.push(`exception: ${JSON.stringify(message.params.exceptionDetails).slice(0, 400)}`);
    }
  });
  const clickButton = (text, within = 'document') =>
    evaluate(`(() => {
      const root = ${within};
      if (!root) return false;
      const button = [...root.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(text)} && !b.matches(':disabled'));
      if (!button) return false;
      button.click();
      return true;
    })()`);
  const goto = async (hash) => {
    await evaluate(`location.hash = ${JSON.stringify(hash)}`);
    await sleep(500);
  };
  const pageHas = (text) => `document.body.innerText.includes(${JSON.stringify(text)})`;

  // IndexedDB, read from the page's own origin (a second connection; no upgrade).
  const idbAll = (store) =>
    evaluate(`new Promise((resolve, reject) => {
      const open = indexedDB.open('bruhs');
      open.onerror = () => reject(String(open.error));
      open.onsuccess = () => {
        const db = open.result;
        try {
          const all = db.transaction(${JSON.stringify(store)}, 'readonly').objectStore(${JSON.stringify(store)}).getAll();
          all.onsuccess = () => { db.close(); resolve(JSON.parse(JSON.stringify(all.result, (k, v) => typeof v === 'bigint' ? v.toString() : v))); };
          all.onerror = () => { db.close(); reject(String(all.error)); };
        } catch (error) { db.close(); reject(String(error)); }
      };
    })`);
  const idbPut = (store, value) =>
    evaluate(`new Promise((resolve, reject) => {
      const open = indexedDB.open('bruhs');
      open.onerror = () => reject(String(open.error));
      open.onsuccess = () => {
        const db = open.result;
        const tx = db.transaction(${JSON.stringify(store)}, 'readwrite');
        tx.objectStore(${JSON.stringify(store)}).put(${JSON.stringify(value)});
        tx.oncomplete = () => { db.close(); resolve(true); };
        tx.onerror = () => { db.close(); reject(String(tx.error)); };
      };
    })`);
  const metadataValue = async (key) => {
    const row = (await idbAll('metadata')).find((entry) => entry.key === key);
    if (!row) return null;
    let value = row.value;
    if (typeof value === 'string') {
      try {
        value = JSON.parse(value);
      } catch {}
    }
    return { revision: row.revision, value, raw: row };
  };
  const savedWorkflow = async (id) => {
    const state = await metadataValue('bruhs.automationState');
    return state?.value?.workflows?.find((workflow) => workflow.id === id) ?? null;
  };

  // ── MCP ────────────────────────────────────────────────────────────────────
  function startMcp(connectionPath) {
    const child = spawn(mcpBin, [], {
      env: { PATH: '/usr/bin:/bin', HOME: path.join(runDir, 'home'), BRUHS_WORKSPACE_BRIDGE: connectionPath },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    children.add(child);
    let buffer = '';
    const waiting = [];
    child.stdout.on('data', (chunk) => {
      buffer += chunk;
      let index;
      while ((index = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, index);
        buffer = buffer.slice(index + 1);
        const next = waiting.shift();
        if (next) next(JSON.parse(line));
      }
    });
    child.stderr.on('data', (chunk) => fs.appendFileSync(path.join(runDir, 'mcp-stderr.log'), chunk));
    let id = 1;
    const rpc = (method, params) =>
      new Promise((resolve) => {
        waiting.push(resolve);
        child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: id++, method, params })}\n`);
      });
    const call = async (name, args = {}) => {
      const response = await rpc('tools/call', { name, arguments: args });
      const result = response.result ?? {};
      const text = result.content?.[0]?.text ?? JSON.stringify(response.error ?? '');
      let value = null;
      try {
        value = JSON.parse(text);
      } catch {}
      return { isError: result.isError === true || !!response.error, text, value, raw: response };
    };
    return { child, rpc, call, stop: () => child.stdin.end() };
  }

  const sessionBodies = [];
  const requestMeta = new Map();
  listeners.push((message) => {
    if (message.method === 'Network.requestWillBeSent') {
      requestMeta.set(message.params.requestId, { url: message.params.request.url, method: message.params.request.method });
    }
  });
  listeners.push(async (message) => {
    if (message.method === 'Network.loadingFinished') {
      const meta = requestMeta.get(message.params.requestId);
      if (meta && meta.url.endsWith('/api/workspace-bridge/session') && meta.method === 'POST') {
        try {
          sessionBodies.push((await send('Network.getResponseBody', { requestId: message.params.requestId })).body);
        } catch {}
      }
    }
  });

  async function enableAndCopy(label) {
    await goto('#/automations');
    const toggleReady = await waitFor(`(() => { const b = document.querySelector('[data-testid=mcp-connection-toggle]'); return b && !b.disabled && b.textContent.trim() === 'Enable for this session'; })()`);
    record(`${label}: Enable is available once the live stream says hello`, toggleReady);
    await evaluate(`document.querySelector('[data-testid=mcp-connection-toggle]').click()`);
    record(`${label}: session enabled`, await waitFor(`!!document.querySelector('[data-testid=mcp-copy-config]')`));
    await evaluate(`document.querySelector('[data-testid=mcp-copy-config]').click()`);
    await sleep(300);
    let connectionPath = null;
    try {
      const copied = await evaluate(`navigator.clipboard.readText()`);
      const config = JSON.parse(copied);
      connectionPath = config.mcpServers.bruhs.env.BRUHS_WORKSPACE_BRIDGE;
      record(`${label}: copied config names a connection file and no token`, typeof connectionPath === 'string' && !/[0-9a-f]{64}/.test(copied));
    } catch (error) {
      record(`${label}: clipboard holds the MCP config`, false, error.message);
    }
    await sleep(300);
    const body = sessionBodies.at(-1);
    if (!connectionPath && body) connectionPath = JSON.parse(body).connection_path;
    const stat = fs.statSync(connectionPath);
    record(`${label}: connection file is private (0600)`, (stat.mode & 0o777) === 0o600, (stat.mode & 0o777).toString(8));
    return connectionPath;
  }

  async function setGrant(label, access, on) {
    const selector = `[aria-label="Toggle ${label} ${access} access"]`;
    await goto('#/automations');
    await waitFor(`!!document.querySelector(${JSON.stringify(selector)})`);
    const pressed = await evaluate(`document.querySelector(${JSON.stringify(selector)})?.getAttribute('aria-pressed')`);
    if ((pressed === 'true') !== on) await evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
    const ok = await waitFor(`document.querySelector(${JSON.stringify(selector)})?.getAttribute('aria-pressed') === ${JSON.stringify(String(on))}`);
    // Grants are saved metadata; wait until the save lands so the tab answers from it.
    await sleep(600);
    return ok;
  }

  // ── Start ──────────────────────────────────────────────────────────────────
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Network.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  await send('Emulation.setFocusEmulationEnabled', { enabled: true });
  try {
    await send('Browser.grantPermissions', { origin: base, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'] });
  } catch (error) {
    console.log(`clipboard permission not granted: ${error.message}`);
  }
  await send('Page.navigate', { url: `${base}/#/automations` });
  record('Agent workflows loads', await waitFor(`!!document.querySelector('#mcp-connection-title')`, 30000));

  // Seed what only the tab stores (IndexedDB), then reload so the app reads it:
  // Ada's existing relationship profile, and a chat whose second turn was
  // stopped (stored with the marker) next to a row from before the marker.
  const now = Date.now();
  const adaProfile = {
    person_id: 'person-ada',
    company: 'Analytical Engines',
    role: 'Collaborator',
    email: '',
    phone: '',
    location: '',
    notes: 'Prefers written briefs.',
    items: [{ id: 'item-ada-1', kind: 'commitment', text: 'Share the launch outline', due_at_ms: null, completed_at_ms: null, source_record_id: null, created_at_ms: now - 3600000 }],
    last_contacted_at_ms: now - 7200000,
    updated_at_ms: now - 3600000,
  };
  await idbPut('relationships', adaProfile);
  await idbPut('threads', { id: 'thread-stopped', title: 'Long task thread', project_name: '', updated_at: now - 60000 });
  await idbPut('messages', {
    thread_id: 'thread-stopped',
    messages: [
      { id: 'u1', role: 'user', is_streaming: false, started_at_ms: now - 120000, completed_at_ms: now - 120000, parts: [{ kind: 'text', value: 'Summarize the logs' }] },
      { id: 'a1', role: 'assistant', is_streaming: false, started_at_ms: now - 119000, completed_at_ms: now - 110000, model: 'claude-opus', parts: [{ kind: 'text', value: 'The logs show two restarts.' }] },
      { id: 'u2', role: 'user', is_streaming: false, started_at_ms: now - 100000, completed_at_ms: now - 100000, parts: [{ kind: 'text', value: 'Now run the long task' }] },
      { id: 'a2', role: 'assistant', is_streaming: false, started_at_ms: now - 99000, completed_at_ms: now - 95000, model: 'claude-opus', stopped: true, parts: [{ kind: 'text', value: 'Starting the long' }] },
    ],
  });
  await send('Page.reload');
  await sleep(1500);
  record('Agent workflows reloads with the seeded records', await waitFor(`!!document.querySelector('#mcp-connection-title')`, 30000));

  const connectionPath = await enableAndCopy('session');
  await screenshot('01-session-on.png');
  const mcp = startMcp(connectionPath);
  await mcp.rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'e2e', version: '1' } });
  const tools = (await mcp.rpc('tools/list', {})).result.tools.map((tool) => tool.name);
  record('tools/list is the connected catalog (28 tools)', tools.length === 28 && tools.includes('bruhs_write_health'), `${tools.length} tools`);

  // Wait for the tab's stores to finish loading: retry a read while it reports loading.
  const retry = async (name, args, done = (r) => !r.isError || !r.text.includes('still loading')) => {
    let result;
    for (let attempt = 0; attempt < 60; attempt++) {
      result = await mcp.call(name, args);
      if (done(result)) return result;
      await sleep(250);
    }
    return result;
  };

  // ── People and Meetings reads over real records ─────────────────────────────
  group = 'people-read';
  let people = await retry('bruhs_read_people', {}, (r) => !r.isError && (r.value?.records?.length ?? 0) >= 2);
  const ada = people.value?.records?.find((p) => p.id === 'person-ada');
  record('bruhs_read_people lists the seeded contacts', ada?.display_name === 'Ada Lovelace' && people.value.records.some((p) => p.id === 'person-grace'), people.text);
  people = await mcp.call('bruhs_read_people', { query: 'grace' });
  record('a query matches by name', people.value?.records?.length === 1 && people.value.records[0].id === 'person-grace', people.text);
  const profile = await mcp.call('bruhs_read_person', { person_id: 'person-ada' });
  record('bruhs_read_person returns the profile and relationship', profile.value?.records?.[0]?.relationship?.company === 'Analytical Engines' && profile.value.records[0].display_name === 'Ada Lovelace', profile.text);
  const items = await mcp.call('bruhs_read_person', { person_id: 'person-ada', section: 'relationship_items' });
  record('relationship_items pages the stored items', items.value?.records?.length === 1 && items.value.records[0].text === 'Share the launch outline', items.text);
  const graceProfile = await mcp.call('bruhs_read_person', { person_id: 'person-grace' });
  record('a contact without context reads relationship: null', !graceProfile.isError && graceProfile.value?.records?.[0]?.relationship === null, graceProfile.text);
  const missingPerson = await mcp.call('bruhs_read_person', { person_id: 'person-nobody' });
  record('an unknown person ID is an explicit error', missingPerson.isError, missingPerson.text);
  await setGrant('People', 'Read', false);
  const peopleRevoked = await mcp.call('bruhs_read_person', { person_id: 'person-ada' });
  record('revoking People Read refuses the one-person read', peopleRevoked.isError && peopleRevoked.text === 'Read access is not granted for this domain.', peopleRevoked.text);
  await setGrant('People', 'Read', true);

  group = 'meetings-read';
  let meetings = await retry('bruhs_read_meetings', {}, (r) => !r.isError && (r.value?.records?.length ?? 0) >= 1);
  const meeting = meetings.value?.records?.find((m) => m.id === meetingId);
  record('bruhs_read_meetings lists the seeded meeting', meeting?.title === 'Launch sync with Ada' && meeting.summary_available === true, meetings.text);
  const summary = await mcp.call('bruhs_read_meeting', { meeting_id: meetingId });
  record('bruhs_read_meeting returns the stored summary and speakers', summary.value?.records?.[0]?.summary?.tldr === 'Ada and I agreed the launch brief ships Friday.' && Array.isArray(summary.value.records[0].speakers), summary.text);
  let transcript = await mcp.call('bruhs_read_meeting', { meeting_id: meetingId, section: 'transcript' });
  record('an unopened transcript is an explicit error, not an empty page', transcript.isError, transcript.text);
  await goto(`#/meetings/${meetingId}`);
  await waitFor(pageHas('Happy to. The brief is nearly done.'), 20000);
  transcript = await retry('bruhs_read_meeting', { meeting_id: meetingId, section: 'transcript' }, (r) => !r.isError);
  record('after opening the meeting, the transcript pages its segments in order', transcript.value?.records?.length === 3 && transcript.value.records[1].text === 'Happy to. The brief is nearly done.' && transcript.value.records[0].partial === false, transcript.text);
  await screenshot('02-meeting-opened.png');
  await goto('#/automations');

  // ── Chat message records report the recorded stop ──────────────────────────
  group = 'chat-stopped';
  const chatMessages = await retry('bruhs_read_chat_messages', { thread_id: 'thread-stopped' }, (r) => !r.isError);
  const records = chatMessages.value?.records ?? [];
  record('the seeded thread reads four completed messages', records.length === 4, chatMessages.text);
  record('a stopped turn reports stopped: true', records[3]?.stopped === true && records[3]?.error === null && records[3]?.content === 'Starting the long', JSON.stringify(records[3]));
  record('a row saved without the marker reports stopped: false', records[1]?.stopped === false, JSON.stringify(records[1]));

  // ── Write domains ──────────────────────────────────────────────────────────
  const day = new Date();
  day.setHours(15, 0, 0, 0);
  const eventStart = day.getTime();
  const domains = [
    {
      key: 'tasks', label: 'Tasks',
      operation: { type: 'create_task', title: 'E2E launch task' },
      payloadText: 'E2E launch task',
      saved: async () => (await idbAll('work_items')).filter((t) => t.title === 'E2E launch task'),
      ui: { hash: '#/tasks', text: 'E2E launch task' },
      read: ['bruhs_read_tasks', { query: 'E2E launch task' }, (r) => r.title === 'E2E launch task'],
    },
    {
      key: 'chat', label: 'Chat',
      operation: { type: 'create_chat_thread', title: 'E2E review chat' },
      payloadText: 'E2E review chat',
      saved: async () => (await idbAll('threads')).filter((t) => t.title === 'E2E review chat'),
      ui: { hash: '#/chat', text: 'E2E review chat' },
      read: ['bruhs_read_chat', { query: 'E2E review chat' }, (r) => r.title === 'E2E review chat'],
    },
    {
      key: 'calendar', label: 'Calendar',
      operation: { type: 'create_calendar_event', title: 'E2E planning block', start_at_ms: eventStart, end_at_ms: eventStart + 3600000 },
      payloadText: 'E2E planning block',
      saved: async () => (await idbAll('events')).filter((e) => e.title === 'E2E planning block'),
      ui: { hash: '#/calendar', text: 'E2E planning block' },
      read: ['bruhs_read_calendar', { query: 'E2E planning block' }, (r) => r.title === 'E2E planning block'],
    },
    {
      key: 'capture', label: 'Capture',
      operation: { type: 'add_capture', content: 'E2E captured thought about pricing' },
      payloadText: 'E2E captured thought about pricing',
      saved: async () => (await idbAll('captures')).filter((c) => `${c.title} ${c.body}`.includes('E2E captured thought')),
      ui: { search: 'E2E captured thought', text: 'E2E captured thought' },
      read: ['bruhs_read_capture', { query: 'E2E captured thought' }, (r) => JSON.stringify(r).includes('E2E captured thought about pricing')],
    },
    {
      key: 'knowledge', label: 'Knowledge',
      operation: { type: 'create_knowledge_note', title: 'E2E knowledge note', body: 'The brief ships Friday.' },
      payloadText: 'The brief ships Friday.',
      saved: async () => (await idbAll('knowledge_documents')).filter((d) => d.title === 'E2E knowledge note'),
      ui: { hash: '#/knowledge', text: 'E2E knowledge note' },
      read: ['bruhs_read_knowledge', { query: 'E2E knowledge note' }, (r) => r.title === 'E2E knowledge note'],
    },
    {
      key: 'inbox', label: 'Inbox',
      operation: { type: 'draft_inbox_message', recipient: 'E2E Recipient', body: 'The outline is ready for review.' },
      payloadText: 'The outline is ready for review.',
      saved: async () => (await idbAll('inbox_conversations')).filter((c) => c.subject === 'Draft to E2E Recipient'),
      ui: { hash: '#/inbox', text: 'Draft to E2E Recipient' },
      read: ['bruhs_read_inbox', { query: 'E2E Recipient' }, (r) => r.subject === 'Draft to E2E Recipient'],
    },
    {
      key: 'people', label: 'People',
      operation: { type: 'create_person_reminder', person_id: 'person-ada', person: 'Ada Lovelace', reminder: 'Send Ada the e2e follow-up' },
      payloadText: 'Send Ada the e2e follow-up',
      saved: async () => (await idbAll('relationships')).filter((p) => p.person_id === 'person-ada' && p.items.some((i) => i.text === 'Send Ada the e2e follow-up')),
      restored: async () => {
        const profile = (await idbAll('relationships')).find((p) => p.person_id === 'person-ada');
        return JSON.stringify(profile) === JSON.stringify(adaProfile) ? null : JSON.stringify(profile);
      },
      ui: { hash: '#/people', person: 'Ada Lovelace', personId: 'person-ada', text: 'Send Ada the e2e follow-up' },
      read: ['bruhs_read_person', { person_id: 'person-ada', section: 'relationship_items' }, (r) => r.text === 'Send Ada the e2e follow-up' && r.kind === 'reminder'],
    },
    {
      key: 'people-new', label: 'People',
      operation: { type: 'create_person_reminder', person_id: 'person-grace', person: 'Grace Hopper', reminder: 'Ask Grace about the compiler talk' },
      payloadText: 'Ask Grace about the compiler talk',
      saved: async () => (await idbAll('relationships')).filter((p) => p.person_id === 'person-grace'),
      ui: { hash: '#/people', person: 'Grace Hopper', personId: 'person-grace', text: 'Ask Grace about the compiler talk' },
      read: ['bruhs_read_person', { person_id: 'person-grace', section: 'relationship_items' }, (r) => r.text === 'Ask Grace about the compiler talk'],
    },
    {
      key: 'meetings', label: 'Meetings',
      operation: { type: 'create_meeting_note', title: 'E2E meeting note', body: 'Decided: ship Friday.' },
      payloadText: 'Decided: ship Friday.',
      saved: async () => (await idbAll('meeting_notes')).filter((n) => n.title === 'E2E meeting note'),
      ui: { hash: '#/meetings', text: 'E2E meeting note' },
      read: ['bruhs_read_meeting_notes', { query: 'E2E meeting note' }, (r) => r.title === 'E2E meeting note'],
    },
    {
      key: 'finance', label: 'Finance',
      operation: { type: 'add_finance_transaction', merchant: 'E2E Cafe', category: 'Dining', amount_minor: '-1234' },
      payloadText: 'E2E Cafe',
      saved: async () => ((await metadataValue('bruhs.lifeContext'))?.value?.finance?.transactions ?? []).filter((t) => t.merchant === 'E2E Cafe'),
      ui: { life: 'Finance', text: 'E2E Cafe' },
      read: ['bruhs_read_finance', { query: 'E2E Cafe' }, (r) => r.merchant === 'E2E Cafe'],
      consent: 'Finance',
    },
    {
      key: 'health', label: 'Health',
      operation: { type: 'log_workout', activity: 'E2E walk', minutes: 37 },
      payloadText: 'E2E walk',
      saved: async () => ((await metadataValue('bruhs.lifeContext'))?.value?.health?.workouts ?? []).filter((w) => w.activity === 'E2E walk'),
      ui: { life: 'Health', text: 'E2E walk' },
      read: ['bruhs_read_health', { query: 'E2E walk' }, (r) => r.activity === 'E2E walk' && r.duration_minutes === 37],
      consent: 'Health',
    },
  ];

  async function setLifeAgentAccess(section, on) {
    await goto('#/life');
    await waitFor(`!!document.querySelector('#life-title')`);
    await clickButton(section);
    await sleep(400);
    const label = on ? 'Agent access off' : 'Agent access on';
    await clickButton(label);
    return waitFor(pageHas(on ? 'Agent access on' : 'Agent access off'), 5000);
  }

  async function showsInUi(ui) {
    if (ui.search) {
      await goto('#/search');
      await waitFor(`!!document.querySelector('[aria-label="Search everything"]')`);
      await evaluate(`document.querySelector('[aria-label="Search everything"]').focus()`);
      await evaluate(`(() => { const input = document.querySelector('[aria-label="Search everything"]'); input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true })); })()`);
      await send('Input.insertText', { text: ui.search });
      await sleep(800);
      return evaluate(`(document.querySelector('#search-results')?.innerText ?? '').includes(${JSON.stringify(ui.text)})`);
    }
    if (ui.life) {
      await goto('#/life');
      await waitFor(`!!document.querySelector('#life-title')`);
      await clickButton(ui.life);
      await sleep(600);
      return evaluate(pageHas(ui.text));
    }
    await goto(ui.hash);
    if (ui.person) {
      const row = `document.getElementById(${JSON.stringify(`person-record-${ui.personId}`)})`;
      await waitFor(`!!${row}`, 15000);
      await evaluate(`(() => { const b = ${row}; if (b && b.getAttribute('aria-expanded') !== 'true') b.click(); return !!b; })()`);
      await waitFor(`${row}?.getAttribute('aria-expanded') === 'true'`, 5000);
      await sleep(500);
    }
    return waitFor(pageHas(ui.text), 8000);
  }
  async function goneFromUi(ui) {
    const shown = await showsInUi({ ...ui });
    return !shown;
  }

  const domainSummary = [];
  for (const domain of domains) {
    if (only && !only.split(',').includes(domain.key)) continue;
    group = domain.key;
    const tool = `bruhs_write_${domain.label.toLowerCase()}`;
    const requestId = `e2e-${domain.key}-1`;
    const args = { operation: domain.operation, dry_run: false, request_id: requestId };
    const checks = [];
    const check = (name, pass, detail) => {
      checks.push(record(name, pass, detail));
      return pass;
    };
    try {
      if (domain.consent) {
        await setGrant(domain.label, 'Read', true);
        check('the user turns on Life agent access for the read', await setLifeAgentAccess(domain.consent, true));
      }
      await setGrant(domain.label, 'Write', false);
      const preview = await mcp.call(tool, { ...args, dry_run: true });
      check('a dry run without the grant reports blocked and saves nothing', !preview.isError && preview.value?.allowed === false && preview.value?.executed === false, preview.text);
      const refused = await mcp.call(tool, args);
      check('a proposal without the Write grant is refused', refused.isError && refused.text.includes('Write access is not granted'), refused.text);
      check('no workflow was created by the refusal', !(await metadataValue('bruhs.automationState'))?.value?.workflows?.some((w) => w.steps?.some((s) => JSON.stringify(s.operation).includes(domain.payloadText))), '');
      check('the user grants Write in the tab', await setGrant(domain.label, 'Write', true));
      const proposed = await mcp.call(tool, args);
      const workflowId = proposed.value?.workflow_id;
      check('the proposal is admitted for review', !proposed.isError && proposed.value?.workflow_status === 'previewed' && proposed.value?.requires_user_approval === true, proposed.text);
      const repeated = await mcp.call(tool, args);
      check('a retry with the same request_id returns the same workflow', !repeated.isError && repeated.value?.workflow_id === workflowId, repeated.text);
      const altered = await mcp.call(tool, { ...args, operation: { ...domain.operation, ...(domain.operation.title ? { title: 'Changed' } : {}), ...(domain.operation.reminder ? { reminder: 'Changed' } : {}), ...(domain.operation.content ? { content: 'Changed' } : {}), ...(domain.operation.body && !domain.operation.title ? { body: 'Changed' } : {}), ...(domain.operation.merchant ? { merchant: 'Changed' } : {}), ...(domain.operation.activity ? { activity: 'Changed' } : {}) } });
      check('a changed payload under the same request_id is refused', altered.isError, altered.text);
      await goto('#/automations');
      const reviewShown = await waitFor(`[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].some((s) => s.innerText.includes(${JSON.stringify(domain.payloadText)}) && s.innerText.includes('Proposed by the connected MCP client'))`);
      check('Agent workflows shows the proposal with its full payload', reviewShown);
      await evaluate(`document.querySelector('[data-testid=workflow-awaiting-review]')?.scrollIntoView({ block: 'start' })`);
      await screenshot(`${domain.key}-1-review.png`);
      check('nothing is saved before the user starts it', (await domain.saved()).length === 0);
      check('the user starts the transaction', await clickButton('Start transaction', `[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].find((s) => s.innerText.includes(${JSON.stringify(domain.payloadText)}))`));
      const stepReady = await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`);
      await sleep(300);
      check('Run next step waits for the user', stepReady && (await clickButton('Run next step')));
      let status = null;
      let savedAtDurable = null;
      let checkpointAtDurable = null;
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.executed) {
          // Durable means saved: the record and its checkpoint are in IndexedDB now.
          savedAtDurable = await domain.saved();
          checkpointAtDurable = await savedWorkflow(workflowId);
          break;
        }
        await sleep(200);
      }
      check('status reports executed only with a durable save', status.value?.executed === true && status.value?.durable === true && status.value?.progress?.durable_completed_steps === 1, status.text);
      const created = savedAtDurable ?? [];
      const step = checkpointAtDurable?.steps?.[0];
      check('when status says durable, IndexedDB holds the record and its checkpoint', created.length === 1 && checkpointAtDurable?.status === 'completed' && step?.status === 'completed' && typeof step?.target_id === 'string', JSON.stringify({ records: created.length, workflow: checkpointAtDurable?.status, step: step?.status, target: step?.target_id }));
      const errorShown = await evaluate(`[...document.querySelectorAll('[role=alert]')].map((e) => e.innerText).filter(Boolean).join(' | ')`);
      if (errorShown) console.log(`alerts: ${errorShown}`);
      const [readTool, readArgs, readMatch] = domain.read;
      const live = await mcp.call(readTool, readArgs);
      check(`${readTool} reads the created record`, !live.isError && (live.value?.records ?? []).some(readMatch), live.text);
      if (domain.consent === 'Finance') {
        const redacted = (live.value?.records ?? []).find(readMatch);
        check('amounts stay redacted until the user shares them', redacted && !('amount_minor' in redacted), JSON.stringify(redacted));
        await goto('#/life');
        await clickButton('Finance');
        await sleep(300);
        await clickButton('Amounts redacted');
        await waitFor(pageHas('Exact amounts shared'), 5000);
        await sleep(400);
        const exact = (await mcp.call(readTool, readArgs)).value?.records?.find(readMatch);
        check('with exact amounts shared, the amount reads as the exact decimal string', exact?.amount_minor === '-1234' && exact?.currency === 'USD', JSON.stringify(exact));
      }
      check('the record shows in its page', await showsInUi(domain.ui));
      await screenshot(`${domain.key}-2-created.png`);
      await goto('#/automations');

      const undoPreview = await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId });
      check('undo preview finds one eligible step', undoPreview.value?.eligible === true && undoPreview.value?.completed_steps === 1 && undoPreview.value?.rollback_review === null, undoPreview.text);
      const undoRequest = await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId, dry_run: false });
      check('an undo request opens a review and deletes nothing', undoRequest.value?.rollback_review === 'pending' && (await domain.saved()).length === 1, undoRequest.text);
      await goto('#/automations');
      const undoShown = await waitFor(`!!document.querySelector('[data-testid=mcp-rollback-review]')`);
      check('the tab shows the undo request', undoShown);
      await evaluate(`document.querySelector('[data-testid=mcp-rollback-review]')?.scrollIntoView({ block: 'start' })`);
      await screenshot(`${domain.key}-3-undo-review.png`);
      check('the user approves undo', await clickButton('Undo transaction', `document.querySelector('[data-testid=mcp-rollback-review]')`));
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.rolled_back) break;
        await sleep(200);
      }
      const alerts = await evaluate(`[...document.querySelectorAll('[role=alert],[role=status]')].map((e) => e.innerText).filter(Boolean).join(' | ')`);
      check('status reports the saved undo (durable rolled_back)', status.value?.rolled_back === true && status.value?.rollback_review === 'approved' && status.value?.durable === true && status.value?.progress?.durable_reverted_steps === 1, `${status.text} ${alerts ? `alerts: ${alerts}` : ''}`);
      const savedAfter = await savedWorkflow(workflowId);
      check('IndexedDB holds the rolled-back checkpoint', savedAfter?.status === 'rolled_back', savedAfter?.status);
      if (domain.restored) {
        const diff = await domain.restored();
        check('the prior relationship profile is restored exactly', diff === null, diff ?? '');
      } else {
        check('the created record is gone from IndexedDB', (await domain.saved()).length === 0);
      }
      check('the record is gone from its page', await goneFromUi(domain.ui));
      const after = await mcp.call(readTool, readArgs);
      check(`${readTool} no longer finds it`, !after.isError && !(after.value?.records ?? []).some(readMatch), after.text);
      if (domain.consent) {
        await setLifeAgentAccess(domain.consent, false);
        await setGrant(domain.label, 'Read', false);
        const refusedRead = await mcp.call(readTool, readArgs);
        check('with Read and Life access off again, the read is refused', refusedRead.isError, refusedRead.text);
      }
      await goto('#/automations');
      await setGrant(domain.label, 'Write', false);
    } catch (error) {
      check('domain run completed without a driver error', false, error.stack);
    }
    domainSummary.push({ domain: domain.key, passed: checks.filter(Boolean).length, total: checks.length });
  }

  // ── Negative cases ─────────────────────────────────────────────────────────
  const runOnly = (key) => !only || only.split(',').includes(key);
  if (runOnly('revoked-midway')) {
    group = 'revoked-midway';
    try {
      await setGrant('Knowledge', 'Write', true);
      const args = { operation: { type: 'create_knowledge_note', title: 'E2E revoked note', body: 'Must not be saved.' }, dry_run: false, request_id: 'e2e-revoked-1' };
      const proposed = await mcp.call('bruhs_write_knowledge', args);
      const workflowId = proposed.value?.workflow_id;
      record('the proposal is admitted', proposed.value?.workflow_status === 'previewed', proposed.text);
      await goto('#/automations');
      await waitFor(`[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].some((s) => s.innerText.includes('Must not be saved.'))`);
      record('the user starts it', await clickButton('Start transaction', `[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].find((s) => s.innerText.includes('Must not be saved.'))`));
      await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`);
      await setGrant('Knowledge', 'Write', false);
      const unavailable = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
      record('revoking Write makes status unavailable to the session', unavailable.isError, unavailable.text);
      await goto('#/automations');
      await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`, 5000);
      const clicked = await clickButton('Run next step');
      await sleep(1500);
      const saved = (await idbAll('knowledge_documents')).filter((d) => d.title === 'E2E revoked note');
      record('Run next step creates nothing without the grant', saved.length === 0, `clicked=${clicked} saved=${saved.length}`);
      const workflow = await savedWorkflow(workflowId);
      record('the saved step stays pending', workflow?.steps?.[0]?.status === 'pending', JSON.stringify(workflow?.steps?.[0]?.status));
      await setGrant('Knowledge', 'Write', true);
      const back = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
      record('re-granting restores status, still not executed', !back.isError && back.value?.executed === false && back.value?.progress?.completed_steps === 0, back.text);
      // Leave nothing unfinished for the next proposals: decline by running and undoing.
      await goto('#/automations');
      await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`);
      await clickButton('Run next step');
      let status;
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.executed) break;
        await sleep(200);
      }
      record('with the grant back, the step runs and is durable', status.value?.executed === true && status.value?.durable === true, status.text);
      await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId, dry_run: false });
      await goto('#/automations');
      await waitFor(`!!document.querySelector('[data-testid=mcp-rollback-review]')`);
      await clickButton('Undo transaction', `document.querySelector('[data-testid=mcp-rollback-review]')`);
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.rolled_back) break;
        await sleep(200);
      }
      record('and undoes cleanly', status.value?.rolled_back === true && (await idbAll('knowledge_documents')).filter((d) => d.title === 'E2E revoked note').length === 0, status.text);
      await setGrant('Knowledge', 'Write', false);
    } catch (error) {
      record('revoked-midway completed without a driver error', false, error.stack);
    }
  }

  if (runOnly('chat-open-undo')) {
    group = 'chat-open-undo';
    try {
      await setGrant('Chat', 'Write', true);
      const proposed = await mcp.call('bruhs_write_chat', { operation: { type: 'create_chat_thread', title: 'E2E opened chat' }, dry_run: false, request_id: 'e2e-chat-open-1' });
      const workflowId = proposed.value?.workflow_id;
      await goto('#/automations');
      await waitFor(`[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].some((s) => s.innerText.includes('E2E opened chat'))`);
      await clickButton('Start transaction', `[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].find((s) => s.innerText.includes('E2E opened chat'))`);
      await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`);
      await clickButton('Run next step');
      let status;
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.executed) break;
        await sleep(200);
      }
      record('the chat is created and durable', status.value?.executed === true, status.text);
      const thread = (await idbAll('threads')).find((t) => t.title === 'E2E opened chat');
      await goto(`#thread/${thread?.id}`);
      await waitFor(`!!document.querySelector('#chat-input')`, 10000);
      await sleep(800);
      const blocked = await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId });
      record('undo preview refuses while the created chat is open', blocked.value?.eligible === false || blocked.isError, blocked.text);
      const afterOpen = (await idbAll('threads')).find((t) => t.id === thread?.id);
      record('opening the chat leaves its saved record unchanged', JSON.stringify(afterOpen) === JSON.stringify(thread), `${JSON.stringify(thread)} -> ${JSON.stringify(afterOpen)}`);
      await goto('#/automations');
      await sleep(500);
      const stillActive = await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId });
      record('Agent workflows alone does not close the chat (it stays the active thread)', stillActive.isError || stillActive.value?.eligible === false, stillActive.text);
      await goto('#thread/thread-stopped');
      await waitFor(pageHas('Summarize the logs'), 10000);
      await sleep(500);
      await goto('#/automations');
      await sleep(500);
      const open = await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId });
      record('after switching to another thread, undo preview finds the chat eligible', open.value?.eligible === true && open.value?.completed_steps === 1, open.text);
      await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId, dry_run: false });
      await waitFor(`!!document.querySelector('[data-testid=mcp-rollback-review]')`);
      await clickButton('Undo transaction', `document.querySelector('[data-testid=mcp-rollback-review]')`);
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.rolled_back) break;
        await sleep(200);
      }
      const alerts = await evaluate(`[...document.querySelectorAll('[role=alert]')].map((e) => e.innerText).filter(Boolean).join(' | ')`);
      record('the closed chat undoes durably', status.value?.rolled_back === true && !(await idbAll('threads')).some((t) => t.title === 'E2E opened chat'), `${status.text} ${alerts}`);
      await setGrant('Chat', 'Write', false);
    } catch (error) {
      record('chat-open-undo completed without a driver error', false, error.stack);
    }
  }

  // ── One workflow across domains ────────────────────────────────────────────
  if (runOnly('mixed')) {
    group = 'mixed';
    try {
      const mixedLabels = ['Tasks', 'Calendar', 'Knowledge', 'People', 'Meetings', 'Finance', 'Health', 'Inbox', 'Capture', 'Chat'];
      for (const label of mixedLabels) await setGrant(label, 'Write', false);
      const operations = [
        { type: 'create_task', title: 'E2E mixed task' },
        { type: 'create_calendar_event', title: 'E2E mixed review', start_at_ms: eventStart + 7200000, end_at_ms: eventStart + 9000000 },
        { type: 'create_knowledge_note', title: 'E2E mixed brief', body: 'Mixed workflow body.' },
        { type: 'create_person_reminder', person_id: 'person-ada', person: 'Ada Lovelace', reminder: 'Mixed: confirm Friday' },
        { type: 'create_meeting_note', title: 'E2E mixed meeting note', body: 'Mixed notes.' },
        { type: 'add_finance_transaction', merchant: 'E2E Mixed Books', category: 'Work', amount_minor: '-9007199254740993' },
        { type: 'log_workout', activity: 'E2E mixed run', minutes: 25 },
        { type: 'draft_inbox_message', recipient: 'E2E Mixed', body: 'Mixed draft body.' },
        { type: 'add_capture', content: 'E2E mixed capture' },
        { type: 'create_chat_thread', title: 'E2E mixed chat' },
      ];
      const proposal = { title: 'E2E mixed workflow', operations, dry_run: false, request_id: 'e2e-mixed-1' };
      const refused = await mcp.call('bruhs_propose_workflow', proposal);
      record('a proposal across all ten domains is refused while any Write grant is missing', refused.isError, refused.text);
      for (const label of mixedLabels.slice(0, -1)) await setGrant(label, 'Write', true);
      const stillRefused = await mcp.call('bruhs_propose_workflow', proposal);
      record('one missing grant (Chat) still refuses the whole batch', stillRefused.isError, stillRefused.text);
      await setGrant('Chat', 'Write', true);
      const proposed = await mcp.call('bruhs_propose_workflow', proposal);
      const workflowId = proposed.value?.workflow_id;
      record('with every grant, the batch is admitted for review', proposed.value?.workflow_status === 'previewed' && proposed.value?.progress?.total_steps === 10, proposed.text);
      await goto('#/automations');
      const shown = await waitFor(`[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].some((s) => ['E2E mixed review', 'Mixed: confirm Friday', 'E2E Mixed Books', '-9007199254740993', 'E2E mixed chat'].every((t) => s.innerText.includes(t)))`);
      record('the review shows every operation, exact amount included', shown);
      await evaluate(`document.querySelector('[data-testid=workflow-awaiting-review]')?.scrollIntoView({ block: 'start' })`);
      await screenshot('mixed-1-review.png');
      record('the user starts it', await clickButton('Start transaction', `[...document.querySelectorAll('[data-testid=workflow-awaiting-review]')].find((s) => s.innerText.includes('E2E mixed review'))`));
      let status;
      for (let stepIndex = 0; stepIndex < 10; stepIndex++) {
        await waitFor(`[...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Run next step' && !b.matches(':disabled'))`, 15000);
        await clickButton('Run next step');
        for (let attempt = 0; attempt < 60; attempt++) {
          status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
          if (status.value?.durable && status.value?.progress?.durable_completed_steps === stepIndex + 1) break;
          await sleep(200);
        }
      }
      const alertsRun = await evaluate(`[...document.querySelectorAll('[role=alert]')].map((e) => e.innerText).filter(Boolean).join(' | ')`);
      record('all ten steps run, each durable before the next', status.value?.executed === true && status.value?.progress?.durable_completed_steps === 10, `${status.text} ${alertsRun}`);
      const life = (await metadataValue('bruhs.lifeContext'))?.value;
      const books = life?.finance?.transactions?.find((t) => t.merchant === 'E2E Mixed Books');
      record('the exact amount beyond 2^53 is saved exactly', books && String(books.amount_minor) === '-9007199254740993', JSON.stringify(books?.amount_minor));
      const ada = (await idbAll('relationships')).find((p) => p.person_id === 'person-ada');
      record('Ada gained the reminder', ada?.items?.some((i) => i.text === 'Mixed: confirm Friday' && i.kind === 'reminder'), JSON.stringify(ada?.items?.map((i) => i.text)));
      await mcp.call('bruhs_request_workflow_rollback', { workflow_id: workflowId, dry_run: false });
      await goto('#/automations');
      await waitFor(`!!document.querySelector('[data-testid=mcp-rollback-review]')`);
      record('the user approves undo', await clickButton('Undo transaction', `document.querySelector('[data-testid=mcp-rollback-review]')`));
      for (let attempt = 0; attempt < 60; attempt++) {
        status = await mcp.call('bruhs_workflow_status', { workflow_id: workflowId });
        if (status.value?.rolled_back) break;
        await sleep(200);
      }
      const alertsUndo = await evaluate(`[...document.querySelectorAll('[role=alert]')].map((e) => e.innerText).filter(Boolean).join(' | ')`);
      record('one undo reverts all ten steps durably', status.value?.rolled_back === true && status.value?.progress?.durable_reverted_steps === 10, `${status.text} ${alertsUndo}`);
      const leftovers = {
        tasks: (await idbAll('work_items')).filter((t) => t.title === 'E2E mixed task').length,
        events: (await idbAll('events')).filter((e) => e.title === 'E2E mixed review').length,
        knowledge: (await idbAll('knowledge_documents')).filter((d) => d.title === 'E2E mixed brief').length,
        notes: (await idbAll('meeting_notes')).filter((n) => n.title === 'E2E mixed meeting note').length,
        inbox: (await idbAll('inbox_conversations')).filter((c) => c.subject === 'Draft to E2E Mixed').length,
        captures: (await idbAll('captures')).filter((c) => `${c.title} ${c.body}`.includes('E2E mixed capture')).length,
        threads: (await idbAll('threads')).filter((t) => t.title === 'E2E mixed chat').length,
        finance: ((await metadataValue('bruhs.lifeContext'))?.value?.finance?.transactions ?? []).filter((t) => t.merchant === 'E2E Mixed Books').length,
        health: ((await metadataValue('bruhs.lifeContext'))?.value?.health?.workouts ?? []).filter((w) => w.activity === 'E2E mixed run').length,
      };
      record('every created record is gone', Object.values(leftovers).every((n) => n === 0), JSON.stringify(leftovers));
      const adaAfter = (await idbAll('relationships')).find((p) => p.person_id === 'person-ada');
      record("Ada's profile is restored exactly", JSON.stringify(adaAfter) === JSON.stringify(adaProfile), JSON.stringify(adaAfter));
      for (const label of mixedLabels) await setGrant(label, 'Write', false);
    } catch (error) {
      record('mixed completed without a driver error', false, error.stack);
    }
  }

  // ── A real Stop in a web turn ───────────────────────────────────────────────
  let liveThreadId = null;
  if (runOnly('chat-stop-live')) {
    group = 'chat-stop-live';
    try {
      await goto('#/chat');
      await waitFor(`!!document.querySelector('[aria-label="New thread (⌘⇧O)"]')`, 10000);
      await evaluate(`document.querySelector('[aria-label="New thread (⌘⇧O)"]').click()`);
      await waitFor(`!!document.querySelector('#chat-input')`, 10000);
      await evaluate(`document.querySelector('#chat-input').focus()`);
      await send('Input.insertText', { text: 'Run the long e2e task' });
      await sleep(300);
      const sent = await evaluate(`(() => { const b = document.querySelector('[aria-label="Send message"]'); if (!b || b.disabled) return false; b.click(); return true; })()`);
      record('the user sends a prompt', sent);
      const streaming = await waitFor(pageHas('Starting the long task'), 20000);
      const stopSelector = '[aria-label="Stop generation"], [aria-label="Interrupt"]';
      const stopShown = await waitFor(`!!document.querySelector(${JSON.stringify(stopSelector)})`, 5000);
      record('the stand-in CLI streams and the turn is running', streaming && stopShown);
      await screenshot('chat-stop-1-running.png');
      record('the user presses Stop', await evaluate(`(() => { const b = document.querySelector(${JSON.stringify(stopSelector)}); b?.click(); return !!b; })()`));
      record('the turn ends', await waitFor(`!document.querySelector(${JSON.stringify(stopSelector)}) && !!document.querySelector('[aria-label="Send message"]')`, 10000));
      await sleep(1500);
      await screenshot('chat-stop-2-stopped.png');
      const threads = await idbAll('threads');
      const messagesRows = await idbAll('messages');
      const row = messagesRows.find((r) => JSON.stringify(r).includes('Run the long e2e task'));
      liveThreadId = row?.thread_id ?? null;
      record('the stopped turn is saved with its stop marker', !!row && (row.payload ?? JSON.stringify(row)).includes('"stopped":true'), (row?.payload ?? JSON.stringify(row ?? {})).slice(0, 300));
      const read = await retry('bruhs_read_chat_messages', { thread_id: liveThreadId }, (r) => !r.isError);
      const reply = read.value?.records?.at(-1);
      record('bruhs_read_chat_messages reports the stopped reply as stopped', reply?.role === 'assistant' && reply?.stopped === true && reply?.error === null && reply?.content === 'Starting the long task', JSON.stringify(reply));
      record('the prompt is not stopped', read.value?.records?.[0]?.stopped === false, JSON.stringify(read.value?.records?.[0]));
      record('the thread is listed by bruhs_read_chat', (await mcp.call('bruhs_read_chat', {})).value?.records?.some((t) => t.id === liveThreadId), threads.map((t) => t.title).join(', '));
    } catch (error) {
      record('chat-stop-live completed without a driver error', false, error.stack);
    }
  }

  // ── Revocation and durability across reload ─────────────────────────────────
  group = 'reload';
  const dialogsBefore = dialogs.length;
  await send('Page.reload');
  await sleep(1500);
  const afterReload = await mcp.call('bruhs_read_tasks', {});
  record('a reload revokes the session', afterReload.isError && afterReload.text === 'Workspace connection is not authorized.', afterReload.text);
  record('a reload removes the connection file', !fs.existsSync(connectionPath));
  record('nothing unsaved or running asked to stay on the page', dialogs.length === dialogsBefore, dialogs.join(' | '));
  await waitFor(`!!document.querySelector('#mcp-connection-title')`, 30000);
  const state = await metadataValue('bruhs.automationState');
  const workflows = state?.value?.workflows ?? [];
  record('after reload every proposed workflow is saved as rolled back', workflows.length > 0 && workflows.every((w) => w.status === 'rolled_back'), workflows.map((w) => `${w.title}:${w.status}`).join(', '));
  mcp.stop();

  if (liveThreadId) {
    group = 'chat-stop-reload';
    const again = await enableAndCopy('after reload');
    const mcp3 = startMcp(again);
    await mcp3.rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'e2e', version: '1' } });
    let read;
    for (let attempt = 0; attempt < 60; attempt++) {
      read = await mcp3.call('bruhs_read_chat_messages', { thread_id: liveThreadId });
      if (!read.isError) break;
      await sleep(250);
    }
    const reply = read.value?.records?.at(-1);
    record('after a reload the saved reply still reports stopped: true', reply?.stopped === true && reply?.content === 'Starting the long task', JSON.stringify(reply));
    const seeded = await mcp3.call('bruhs_read_chat_messages', { thread_id: 'thread-stopped' });
    record('rows saved before the marker still load and report stopped: false', seeded.value?.records?.[1]?.stopped === false && seeded.value?.records?.[3]?.stopped === true, seeded.text.slice(0, 200));
    await evaluate(`document.querySelector('[data-testid=mcp-connection-toggle]').click()`);
    await sleep(800);
    record('disable removes the second connection file', !fs.existsSync(again));
    mcp3.stop();
  }

  fs.writeFileSync(path.join(runDir, 'results.json'), JSON.stringify({ results, domainSummary }, null, 2));
  fs.writeFileSync(path.join(runDir, 'console.log'), consoleLines.join('\n'));
  const failed = results.filter((result) => !result.pass);
  console.log('\nPer domain:');
  for (const row of domainSummary) console.log(`  ${row.domain}: ${row.passed}/${row.total}`);
  console.log(`${results.length - failed.length}/${results.length} checks passed`);
  ws.close();
  exitCode = failed.length ? 1 : 0;
} catch (error) {
  console.error(error);
} finally {
  await cleanup();
}
process.exit(exitCode);
