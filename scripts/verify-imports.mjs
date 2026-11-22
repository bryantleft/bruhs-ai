// Acceptance run for imports on the web (docs/imports.md; PARITY-22 in
// docs/web-desktop-parity-goal-2026-11-15.md): one scratch home with a real
// bruhs-server, chat service exports this script builds itself (a ChatGPT
// export in two shards with its manifest and a users.json canary, a Claude.ai
// export with conversations and the newer memory files, Grok's bare JSON and
// a hostile ZIP), an OpenClaw workspace and a Hermes Agent home with secret
// canaries beside their persona files, and a bruh made through the API. It
// covers the Import thread dialog's Export file tab (tabs, help lines,
// uploads, progress and batch lines, best effort rows, Import all, each
// thread's transcript and its first turn's request, ALREADY_HERE), the
// owner-only routes, the hostile ZIP, the batch folder's modes and its
// discard on close, Claude.ai memory into By prompt, Move in by keyboard,
// Make this bruh and the memory proposal (Decline, then Approve on a second
// import), the canaries in every answer and the log, Copy prompt and the
// stale warning, Hermes entries, phone-size touch, reduced motion and
// panics. Every check prints ok or FAIL; the exit status is 0 only when all
// pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && CARGO_INCREMENTAL=0 BRUHS_API_URL=http://127.0.0.1:43091 trunk build --dist "$TMPDIR/bruhs-imports-dist")
//   node scripts/verify-imports.mjs "$PWD" "$TMPDIR/bruhs-imports-dist"
//
// The server is the debug target/debug/bruhs-server on port 43091 with a
// scratch HOME, BRUHS_SECRET_STORE=file:<home>/keys (so the owner's keychain
// is never touched), no BRUHS_* data path overrides, no OPENCLAW_* or
// HERMES_HOME (F34: the environment is built from nothing) and a capture URL
// nothing listens on. Headless Chrome runs on BRUHS_VERIFY_CDP_PORT (default
// 9491) with its own profile. Results, logs and screenshots go to
// $TMPDIR/bruhs-imports-verify/, cleared on each run. Canaries are random;
// nothing this script prints or writes holds one.
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { launchChrome, newContext, newPage, sleep } from '../bruhs-web/tests/cdp_harness.mjs';

const root = path.resolve(process.argv[2] ?? '');
const dist = path.resolve(process.argv[3] ?? '');
const serverBin = path.join(root, 'target/debug/bruhs-server');
const fixtures = path.join(root, 'bruhs-core/src/import/fixtures');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle', path.join(dist, 'index.html')], ['fixtures', fixtures]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
if (process.platform !== 'darwin' && process.platform !== 'linux') {
  console.error('This acceptance run checks unix modes; run it on macOS or Linux.');
  process.exit(2);
}
const PORT = 43091;
const base = `http://127.0.0.1:${PORT}`;
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9491);
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-imports-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const home = path.join(runDir, 'home');
const shots = path.join(runDir, 'shots');
const files = path.join(runDir, 'files');
const outside = path.join(runDir, 'outside');
for (const dir of [home, shots, files, outside, path.join(home, 'tmp')]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
const serverLog = path.join(runDir, 'server.log');
const coreRoot = process.platform === 'darwin' ? path.join(home, 'Library/Application Support/dev.bruhs.bruhs-ai') : path.join(home, '.local/share/bruhs-ai');
const importsDir = path.join(coreRoot, 'imports');
console.log(`Run directory: ${runDir}`);
const js = JSON.stringify;

// ── Canaries ─────────────────────────────────────────────────────────────────
const hex = (bytes = 12) => randomBytes(bytes).toString('hex');
const tag = hex(3);
const canaries = Object.fromEntries(['users', 'escape', 'clawdEnv', 'clawdAuth', 'clawdConfig', 'soulOutside', 'hermesEnv', 'hermesAuth', 'hermesConfig', 'hermesState'].map((name) => [name, `cn-${name}-${hex(12)}`]));
const canarySet = Object.entries(canaries);
const found = (text) => canarySet.filter(([, value]) => String(text ?? '').includes(value)).map(([name]) => name);
const sanitize = (text) => canarySet.reduce((out, [name, value]) => out.replaceAll(value, `[${name}]`), String(text ?? ''));

// ── A minimal ZIP writer: local headers, a central directory, stored and deflate ──
function zipOf(entries) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const { name, data, method = 'deflate' } of entries) {
    const nameBytes = Buffer.from(name, 'utf8');
    const raw = Buffer.isBuffer(data) ? data : Buffer.from(data);
    const deflate = method === 'deflate';
    const body = deflate ? zlib.deflateRawSync(raw, { level: 9 }) : raw;
    const crc = zlib.crc32(raw) >>> 0;
    const fields = (header, at) => {
      header.writeUInt16LE(20, at); // version needed
      header.writeUInt16LE(0x0800, at + 2); // UTF-8 names
      header.writeUInt16LE(deflate ? 8 : 0, at + 4);
      header.writeUInt16LE(0, at + 6); // time
      header.writeUInt16LE(0x5b61, at + 8); // date (2025-11-01)
      header.writeUInt32LE(crc, at + 10);
      header.writeUInt32LE(body.length, at + 14);
      header.writeUInt32LE(raw.length, at + 18);
      header.writeUInt16LE(nameBytes.length, at + 22);
      header.writeUInt16LE(0, at + 24); // extra
    };
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    fields(local, 4);
    locals.push(local, nameBytes, body);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(0x0314, 4); // made by: unix, 2.0
    fields(central, 6);
    central.writeUInt16LE(0, 32); // comment
    central.writeUInt16LE(0, 34); // disk
    central.writeUInt16LE(0, 36); // internal attributes
    central.writeUInt32LE((0o100644 << 16) >>> 0, 38); // a regular file
    central.writeUInt32LE(offset, 42);
    centrals.push(central, nameBytes);
    offset += local.length + nameBytes.length + body.length;
  }
  const directory = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, directory, end]);
}

// ── Seeds ────────────────────────────────────────────────────────────────────
const fixture = (name) => fs.readFileSync(path.join(fixtures, name));
const chatgpt = JSON.parse(fixture('chatgpt-conversations.json'));
const half = Math.ceil(chatgpt.length / 2);
const chatgptZip = path.join(files, 'chatgpt-export.zip');
fs.writeFileSync(chatgptZip, zipOf([
  { name: 'conversations-000.json', data: js(chatgpt.slice(0, half)) },
  { name: 'conversations-001.json', data: js(chatgpt.slice(half)) },
  { name: 'export_manifest.json', data: js({ version: 1, logical_files: { 'conversations.json': { files: ['conversations-000.json', 'conversations-001.json'], sharded: true, shard_count: 2 } } }), method: 'stored' },
  { name: 'users.json', data: js({ id: `user-${tag}`, email: `${canaries.users}@example.invalid`, phone_number: canaries.users }) },
]));
const grokJson = path.join(files, 'prod-grok-backend.json');
fs.writeFileSync(grokJson, fixture('grok-prod-grok-backend.json'));
const claudeZip = path.join(files, 'claude-export.zip');
fs.writeFileSync(claudeZip, zipOf([
  { name: 'conversations.json', data: fixture('claude-conversations.json') },
  { name: `memories/account-${tag}.json`, data: fixture('claude-memories-account.json'), method: 'stored' },
]));
// A deflate bomb: `[` then 96 MiB of spaces, from a few hundred KB.
const bomb = Buffer.alloc(96 << 20, 0x20);
bomb[0] = 0x5b;
const nestedConversations = js(chatgpt.slice(0, 1));
// Three levels inside the upload (ledger: "nesting three deep").
const nested = (levels) => {
  let inner = zipOf([{ name: 'conversations.json', data: nestedConversations }]);
  for (let level = levels; level > 1; level -= 1) inner = zipOf([{ name: `level-${level}.zip`, data: inner, method: 'stored' }]);
  return inner;
};
// A ZIP bomb proper: a nested ZIP whose copy inflates far past its share
// (compressed × MAX_INFLATE_RATIO, at least RATIO_FLOOR_BYTES).
const zipBombBody = Buffer.alloc(96 << 20, 0);
zipBombBody.writeUInt32LE(0x04034b50, 0);
const hostileZip = path.join(files, 'hostile.zip');
fs.writeFileSync(hostileZip, zipOf([
  { name: '../escape.json', data: js({ escape: canaries.escape }), method: 'stored' },
  { name: 'level-1.zip', data: nested(3), method: 'stored' },
  { name: 'inner.zip', data: zipBombBody },
  { name: 'conversations.json', data: bomb },
]));
const bombZip = path.join(files, 'bomb.zip');
fs.writeFileSync(bombZip, zipOf([{ name: 'conversations.json', data: bomb }]));
const zipBombZip = path.join(files, 'zip-bomb.zip');
fs.writeFileSync(zipBombZip, zipOf([{ name: 'inner.zip', data: zipBombBody }]));
const escapeZip = path.join(files, 'escape.zip');
fs.writeFileSync(escapeZip, zipOf([{ name: '../escape.json', data: js({ escape: canaries.escape }), method: 'stored' }, { name: '../../escape-conversations.json', data: nestedConversations }]));
const nested3Zip = path.join(files, 'nested-3.zip');
fs.writeFileSync(nested3Zip, zipOf([{ name: 'level-1.zip', data: nested(3), method: 'stored' }]));
const nested2Zip = path.join(files, 'nested-2.zip');
fs.writeFileSync(nested2Zip, zipOf([{ name: 'level-1.zip', data: nested(2), method: 'stored' }]));

const write = (rel, text, mode = 0o600) => {
  const file = path.join(home, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.writeFileSync(file, text, { mode });
  return file;
};
write('clawd/IDENTITY.md', '# IDENTITY\n\n- **Name:** Pinch\n- **Creature:** crab\n- **Vibe:** calm and dry\n- **Emoji:** 🦀\n');
write('clawd/AGENTS.md', 'Answer in short sentences. Check facts before saying them.\n');
write('clawd/USER.md', 'The owner likes tea and early meetings.\n');
write('clawd/MEMORY.md', `Project Lisbon ships in March. ${tag}\n`);
write('clawd/.env', `OPENAI_API_KEY=${canaries.clawdEnv}\n`);
write('clawd/auth-profiles.json', js({ profiles: { default: { token: canaries.clawdAuth } } }));
write('clawd/openclaw.json', js({ gateway: { token: canaries.clawdConfig } }));
const soulOutside = path.join(outside, 'SOUL.md');
fs.writeFileSync(soulOutside, `Outside soul ${canaries.soulOutside}\n`);
fs.symlinkSync(soulOutside, path.join(home, 'clawd/SOUL.md'));
write('.hermes/SOUL.md', 'You are Hermes. Be brief.\n');
const hermesEntries = ['Prefers metric units.', 'Lives near the sea.', 'Walks the dog at seven.'];
write('.hermes/memories/USER.md', `${hermesEntries[0]}\n§\n${hermesEntries[1]}\n§\n${hermesEntries[0]}\n`);
write('.hermes/memories/MEMORY.md', `${hermesEntries[2]}\n§\n\n`);
write('.hermes/.env', `ANTHROPIC_API_KEY=${canaries.hermesEnv}\n`);
write('.hermes/auth.json', js({ token: canaries.hermesAuth }));
write('.hermes/config.yaml', `api_key: ${canaries.hermesConfig}\n`);
write('.hermes/state.db', `SQLite format 3\0${canaries.hermesState}`);

// ── Files on disk ────────────────────────────────────────────────────────────
const walkFiles = (dir) => {
  const out = [];
  const walk = (folder) => {
    let entries = [];
    try {
      entries = fs.readdirSync(folder, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(folder, entry.name);
      out.push({ full, entry });
      if (entry.isDirectory()) walk(full);
    }
  };
  walk(dir);
  return out;
};
const batches = () => {
  try {
    return fs.readdirSync(importsDir).filter((name) => /^[0-9a-f]{32}$/.test(name));
  } catch {
    return [];
  }
};
const mode = (file) => fs.lstatSync(file).mode & 0o777;
const octal = (value) => `0${value.toString(8)}`;

// ── The server ───────────────────────────────────────────────────────────────
const portBusy = async (port) => {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
    return true;
  } catch (error) {
    return error?.cause?.code !== 'ECONNREFUSED';
  }
};
for (const port of [PORT, cdpPort]) {
  if (await portBusy(port)) {
    console.error(`Port ${port} is in use; free it or set BRUHS_VERIFY_CDP_PORT.`);
    process.exit(2);
  }
}
const children = new Set();
let server = null;
async function startServer() {
  const log = fs.openSync(serverLog, 'a');
  // Built from nothing, so no OPENCLAW_* or HERMES_HOME reaches it (F34).
  const env = {
    PATH: '/usr/bin:/bin:/usr/sbin:/sbin',
    HOME: home,
    TMPDIR: path.join(home, 'tmp'),
    BRUHS_SERVER_PORT: String(PORT),
    BRUHS_WEB_DIST: dist,
    BRUHS_CAPTURE_URL: 'http://127.0.0.1:9',
    BRUHS_SECRET_STORE: `file:${path.join(home, 'keys')}`,
    RUST_LOG: 'info',
  };
  for (const name of ['OPENCLAW_STATE_DIR', 'OPENCLAW_WORKSPACE_DIR', 'OPENCLAW_PROFILE', 'OPENCLAW_CONFIG_PATH', 'HERMES_HOME']) delete env[name];
  server = spawn(serverBin, [], { env, stdio: ['ignore', log, log] });
  fs.closeSync(log);
  children.add(server);
  const started = Date.now();
  for (;;) {
    try {
      if ((await fetch(`${base}/api/health`, { signal: AbortSignal.timeout(1000) })).ok) break;
    } catch {}
    if (server.exitCode !== null) throw new Error(`bruhs-server exited at start (code ${server.exitCode}); see server.log`);
    if (Date.now() - started > 120000) throw new Error('bruhs-server did not answer in 2 minutes');
    await sleep(200);
  }
}
async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise((resolve) => child.once('exit', resolve));
  child.kill('SIGTERM');
  if (await Promise.race([exited.then(() => true), sleep(8000).then(() => false)]) === false) child.kill('SIGKILL');
}
let chrome = null;
let cleaned = false;
async function cleanup() {
  if (cleaned) return;
  cleaned = true;
  if (chrome) await chrome.close().catch(() => {});
  for (const child of [...children].reverse()) await stopChild(child);
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));
process.on('uncaughtException', (error) => {
  console.error(`FAIL uncaught: ${sanitize(error?.stack ?? error)}`);
  cleanup().finally(() => process.exit(1));
});

const results = [];
const record = (name, pass, detail = '') => {
  const shown = sanitize(detail);
  results.push({ name, pass: !!pass, detail: shown.slice(0, 3000) });
  console.log(`${pass ? 'ok  ' : 'FAIL'} ${name}${shown ? ` (${shown.slice(0, 1500)})` : ''}`);
  return !!pass;
};
const check = async (name, body) => {
  try {
    await body();
  } catch (error) {
    record(name, false, error?.stack ?? String(error));
  }
};
const measured = {};

// ── HTTP as the owner (the page's own Origin) or as another page ─────────────
const bodies = [];
const request = async (method, route, { origin = base, json = undefined, form = undefined } = {}) => {
  const headers = { ...(origin ? { origin } : {}) };
  let body;
  if (json !== undefined) {
    headers['content-type'] = 'application/json';
    body = js(json);
  }
  if (form) body = form;
  const response = await fetch(`${base}${route}`, { method, headers, body });
  const text = await response.text();
  bodies.push({ url: route, body: text });
  let parsed = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = text;
  }
  return { status: response.status, body: parsed, text };
};
// A POST the server may refuse before reading its body: node:http, so an
// early close after the answer isn't an error.
const rawPost = (route, origin, body, type) => new Promise((resolve, reject) => {
  const outgoing = http.request(`${base}${route}`, { method: 'POST', headers: { origin, 'content-type': type, 'content-length': body.length } }, (response) => {
    const chunks = [];
    response.on('data', (chunk) => chunks.push(chunk));
    response.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      bodies.push({ url: route, body: text });
      let parsed = null;
      try {
        parsed = JSON.parse(text);
      } catch {}
      resolve({ status: response.statusCode, body: parsed, text });
    });
  });
  outgoing.on('error', (error) => (outgoing.res ? null : reject(error)));
  outgoing.end(body);
});
const api = async (method, route, json) => {
  const answer = await request(method, route, { json });
  if (answer.status < 200 || answer.status > 299) throw new Error(`${method} ${route}: ${answer.status} ${answer.text.slice(0, 300)}`);
  return answer.body;
};
const formOf = (paths) => {
  const form = new FormData();
  for (const file of paths) form.append('file', new Blob([fs.readFileSync(file)]), path.basename(file));
  return form;
};
// An upload through the API, polled to its end, then discarded.
const uploadJob = async (paths) => {
  const started = await request('POST', '/api/import/uploads', { form: formOf(paths) });
  if (started.status !== 202) return { status: started.status, error: started.body?.error ?? started.text };
  const deadline = Date.now() + 120000;
  for (;;) {
    const job = await request('GET', `/api/import/uploads/${started.body.id}`);
    if (job.status !== 200) return { status: job.status, error: job.body?.error ?? job.text };
    // An ImportJob is { id, state: { state, ... } }.
    const { state, message = null, batch = null } = job.body.state ?? {};
    if (state === 'ready' || state === 'failed') {
      const remove = await request('DELETE', `/api/import/uploads/${started.body.id}`);
      return { status: 200, state, message, batch, deleted: remove.status };
    }
    if (Date.now() > deadline) return { status: 0, error: 'the read never ended' };
    await sleep(150);
  }
};
const until = async (label, test, timeout = 15000, interval = 150) => {
  const deadline = Date.now() + timeout;
  for (;;) {
    const value = await test();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`Timed out: ${label}`);
    await sleep(interval);
  }
};

const COPY = {
  tabs: ['Claude Code', 'Codex', 'T3 Code', 'Cursor', 'Export file'],
  helps: [
    'ChatGPT: Settings, Data controls, Export data. Upload the ZIP from the email.',
    'Claude.ai: Settings, Privacy, Export data. Upload the ZIP, or conversations-000.zip and memories-000.zip from the newer list of files.',
    'Grok: Settings, Data controls, Download account data. Upload the ZIP.',
  ],
  grok: "Grok is best effort: xAI doesn't document its export, so some chats may come in incomplete.",
  alreadyHere: 'Every chat in this export is already here.',
  archiveRefused: 'This file holds more than Bruhs reads from an export. Nothing was imported.',
  conversationTooLarge: 'One chat in this export is over 33 MB. Nothing was imported.',
  notAnExport: "Bruhs didn't find ChatGPT, Claude.ai or Grok chats in this file.",
  ownerOnly: 'Open Bruhs from its own address to import.',
  memoryFound: "This export holds Claude.ai's memory of you.",
  skippedLink: "Skipped SOUL.md: it's a link.",
  movePrompt: "I'm moving to a new assistant. List what you know and remember about me that would help it work with me: my preferences, how I like answers written, my work and projects, the people, tools and places I mention, and my routines. Write one fact per line as a plain sentence. Leave out passwords, keys, account numbers and anything I asked you to forget.",
  proposedScout: "Proposed. Approve it on Scout's Memory tab.",
  memoryChanged: "This bruh's memory changed after this was proposed. Approving replaces those changes.",
  imported: 'This thread continues an earlier conversation, which follows; use it as context.',
};

let exitCode = 1;
try {
  await startServer();
  const snapshot = () => api('GET', '/api/reactive');
  const command = (body) => api('POST', '/api/reactive/commands', body);
  const scoutId = `scout-${tag}`;
  await command({
    action: 'create_bruh',
    agent: { id: scoutId, name: 'Scout', instructions: 'Scout watches the calendar.', model: 'claude-sonnet-4-6', project_path: '', enabled: true },
    look: { look: 'left', face: 'blueberry', mouth: 'bruh' },
    access: { level: 'read_only', send_as_me: false, can_deliver: false },
  });

  // ── Chrome ─────────────────────────────────────────────────────────────────
  chrome = await launchChrome({ port: cdpPort, profile: path.join(runDir, 'chrome') });
  const context = await newContext(chrome.browser);
  await chrome.browser.send('Browser.grantPermissions', { origin: base, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'], browserContextId: context.id });
  const pages = [];
  const panics = [];
  const agentRequests = [];
  const frames = `window.__frames = 0; { const request = window.requestAnimationFrame.bind(window); window.requestAnimationFrame = (callback) => { window.__frames += 1; return request(callback); }; }`;
  const watch = async (page) => {
    pages.push(page);
    const responses = new Map();
    page.handlers.add((message) => {
      if (message.method === 'Runtime.exceptionThrown') {
        const text = message.params.exceptionDetails?.exception?.description ?? message.params.exceptionDetails?.text ?? '';
        if (text.includes('panicked') || text.includes('unreachable')) panics.push(text.slice(0, 300));
      }
      if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
        const text = message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ');
        if (text.includes('panicked')) panics.push(text.slice(0, 300));
      }
      if (message.method === 'Network.responseReceived') {
        const url = new URL(message.params.response.url);
        if (url.pathname.startsWith('/api/import/')) responses.set(message.params.requestId, url.pathname);
      }
      if (message.method === 'Network.loadingFinished' && responses.has(message.params.requestId)) {
        const url = responses.get(message.params.requestId);
        responses.delete(message.params.requestId);
        page.send('Network.getResponseBody', { requestId: message.params.requestId })
          .then(({ body, base64Encoded }) => bodies.push({ url, body: base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body, page: true }))
          .catch(() => {});
      }
    });
    await page.send('Network.enable', { maxTotalBufferSize: 64 << 20, maxResourceBufferSize: 16 << 20, maxPostDataSize: 1 << 20 });
    return page;
  };
  const open = async (options = {}) => watch(await newPage(chrome.browser, base, { context, intercept: false, initScripts: [frames], ...options }));
  // Only `POST /api/agent` is held: the harness answers it from `api`, so
  // the turn never reaches a model (the page's own origin is moved aside so
  // the harness treats the held request as a fixture API call).
  const holdAgentTurns = async (page) => {
    page.origin = 'http://held.invalid';
    page.api = async ({ method, url, body, entries }) => {
      if (method !== 'POST' || url.pathname !== '/api/agent') return undefined;
      const text = body ?? (entries ?? []).map((entry) => Buffer.from(entry.bytes ?? '', 'base64').toString('utf8')).join('');
      let parsed = null;
      try {
        parsed = JSON.parse(text);
      } catch {}
      agentRequests.push(parsed);
      return { status: 200, raw: Buffer.from('data: [DONE]\n\n'), headers: [['Content-Type', 'text/event-stream'], ['Cache-Control', 'no-store']] };
    };
    await page.send('Fetch.enable', { patterns: [{ urlPattern: '*/api/agent', requestStage: 'Request' }] });
  };
  const shoot = (page, name) => page.screenshot(path.join(shots, `${name}.png`));
  const q = (selector) => `document.querySelector(${js(selector)})`;
  const texts = (page, selector) => page.evaluate(`[...document.querySelectorAll(${js(selector)})].map((e) => e.innerText.trim())`);
  const clickOn = async (page, expression) => {
    const clicked = await page.click(expression);
    if (!clicked.clicked) throw new Error(`couldn't click ${expression}: ${clicked.reason}`);
  };
  const metaK = async (page) => {
    for (const type of ['keyDown', 'keyUp']) await page.send('Input.dispatchKeyEvent', { type, key: 'k', code: 'KeyK', windowsVirtualKeyCode: 75, modifiers: 4 });
  };
  const palette = async (page, words) => {
    await metaK(page);
    if (!(await page.waitFor(`!!document.querySelector('[role=dialog] input')`, 5000))) throw new Error('⌘K opened no palette');
    await page.type(words);
    await sleep(300);
    await page.press('Enter');
  };
  const importTab = (name) => `[...document.querySelectorAll('[role=tablist][aria-label="Import from"] [role=tab]')].find((t) => t.innerText.trim() === ${js(name)})`;
  const moveTab = (name) => `[...document.querySelectorAll('[role=tablist][aria-label="Move in"] [role=tab]')].find((t) => t.innerText.trim() === ${js(name)})`;
  const openImport = async (page) => {
    await palette(page, 'Import thread');
    if (!(await page.waitFor(`!!(${importTab('Export file')})`, 5000))) throw new Error('Import thread did not open');
  };
  const openExportTab = async (page) => {
    await openImport(page);
    await clickOn(page, importTab('Export file'));
    if (!(await page.waitFor(`!!${q('[data-export-choose]')}`, 5000))) throw new Error('no Export file tab');
  };
  const choose = async (page, paths) => {
    const { root: doc } = await page.send('DOM.getDocument', { depth: 0 });
    const { nodeId } = await page.send('DOM.querySelector', { nodeId: doc.nodeId, selector: '[data-export-tab] input[type=file]' });
    await page.send('DOM.setFileInputFiles', { nodeId, files: paths });
  };
  const closeDialogs = async (page) => {
    for (let i = 0; i < 4; i += 1) {
      if (!(await page.evaluate(`!!document.querySelector('[role=dialog]')`))) return;
      await page.press('Escape');
      await sleep(250);
    }
  };
  const layoutOf = (page, scope) => page.evaluate(`(() => {
    const scope = ${scope};
    const shown = (node) => { const r = node.getBoundingClientRect(); return r.height > 0 && r.width > 0 && !node.closest('.sr-only') && node.getAttribute('aria-hidden') !== 'true'; };
    const label = (node) => node.getAttribute('aria-label') || node.innerText?.trim().split('\\n')[0].slice(0, 30) || node.tagName;
    const controls = scope ? [...scope.querySelectorAll('button, input, select, textarea, [role=tab]')].filter(shown).map((node) => ({ label: label(node), height: Math.round(node.getBoundingClientRect().height) })) : [];
    return { controls, sideways: document.scrollingElement.scrollWidth - innerWidth, dialogSideways: scope ? scope.scrollWidth - scope.clientWidth : null, coarse: matchMedia('(pointer: coarse)').matches };
  })()`);

  const page = await open();
  await page.goto(`${base}/#/`);
  if (!(await page.appReady())) throw new Error("the app didn't load");
  await sleep(1500);
  await holdAgentTurns(page);

  // 1. Five tabs; Export file's help lines and the Grok note; no sideways scroll.
  await check('Import thread shows five tabs; Export file shows the help lines and GROK_BEST_EFFORT; no horizontal scroll at 1280×800 or 390×844', async () => {
    await openImport(page);
    const tabs = await texts(page, '[role=tablist][aria-label="Import from"] [role=tab]');
    await clickOn(page, importTab('Export file'));
    await page.waitFor(`!!${q('[data-export-choose]')}`, 5000);
    const helps = await texts(page, '[data-export-help]');
    const grok = (await texts(page, '[data-export-grok]'))[0];
    const wide = await layoutOf(page, `document.querySelector('[role=dialog]')`);
    await shoot(page, '1-export-start-1280x800');
    await closeDialogs(page);
    const phone = await open({ width: 390, height: 844, touch: true });
    await phone.goto(`${base}/#/`);
    await phone.appReady();
    await sleep(1500);
    await openImport(phone);
    const strip = await phone.evaluate(`(() => { const s = document.querySelector('[role=tablist][aria-label="Import from"]'); return { client: s.clientWidth, scroll: s.scrollWidth, overflow: getComputedStyle(s).overflowX }; })()`);
    await clickOn(phone, importTab('Export file'));
    await phone.waitFor(`!!${q('[data-export-choose]')}`, 5000);
    const narrow = await layoutOf(phone, `document.querySelector('[role=dialog]')`);
    const chosenVisible = await phone.evaluate(`(() => { const t = ${importTab('Export file')}; const s = t.closest('[role=tablist]').getBoundingClientRect(); const r = t.getBoundingClientRect(); return r.left >= s.left - 1 && r.right <= s.right + 1; })()`);
    await shoot(phone, '1-export-start-390x844');
    await phone.close();
    record('Import thread shows five tabs; Export file shows the help lines and GROK_BEST_EFFORT; no horizontal scroll at 1280×800 or 390×844',
      js(tabs) === js(COPY.tabs) && js(helps) === js(COPY.helps) && grok === COPY.grok && wide.sideways <= 0 && wide.dialogSideways <= 0 && narrow.sideways <= 0 && narrow.dialogSideways <= 0 && strip.overflow === 'auto' && chosenVisible,
      `tabs ${js(tabs)}; helps ${helps.length}; grok ${grok === COPY.grok}; 1280 sideways ${wide.sideways}/${wide.dialogSideways}; 390 sideways ${narrow.sideways}/${narrow.dialogSideways}; strip ${js(strip)}; chosen tab in view ${chosenVisible}`);
  });

  // 2 and 6. Upload, progress, batch lines, modes, Import all, transcripts, ALREADY_HERE.
  let imported = [];
  let readyBatch = null;
  await check('Upload ChatGPT parts and Grok: progress lines, batch lines, best effort Grok rows; Import all imports every row', async () => {
    await openExportTab(page);
    // Slow the upload so its reading line can be seen.
    await page.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: 6 * 1024 });
    const seen = new Set();
    let polling = true;
    const poller = (async () => {
      while (polling) {
        try {
          const line = await page.evaluate(`${q('[data-export-status]')}?.innerText.trim() ?? ''`);
          if (line) seen.add(line);
        } catch {}
        await sleep(25);
      }
    })();
    await choose(page, [chatgptZip, grokJson]);
    const ready = await page.waitFor(`!!${q('[data-export-all]')}`, 60000);
    polling = false;
    await poller;
    await page.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
    if (!ready) throw new Error(`no batch; error ${js(await page.evaluate(`${q('[data-export-error]')}?.innerText`))}`);
    const progress = [...seen].filter((line) => line.startsWith('Reading the export'));
    const lines = await texts(page, '[data-export-line]');
    const details = await texts(page, '[data-export-detail]');
    const titles = await texts(page, '[data-export-title]');
    const grokRows = details.filter((detail) => detail.startsWith('Grok'));
    const chatgptRows = details.filter((detail) => detail.startsWith('ChatGPT'));
    measured.progressLines = progress;
    measured.batchLines = lines;
    await shoot(page, '2-export-ready-1280x800');
    const batchIds = batches();
    readyBatch = batchIds.length === 1 ? batchIds[0] : null;
    // Check 6, while the batch is Ready.
    if (readyBatch) {
      const folder = path.join(importsDir, readyBatch);
      const all = walkFiles(folder);
      const badDirs = all.filter(({ entry, full }) => entry.isDirectory() && mode(full) !== 0o700).map(({ full }) => `${path.relative(folder, full)}:${octal(mode(full))}`);
      const badFiles = all.filter(({ entry, full }) => entry.isFile() && mode(full) !== 0o600).map(({ full }) => `${path.relative(folder, full)}:${octal(mode(full))}`);
      const listing = (name) => { try { return fs.readdirSync(path.join(folder, name)); } catch { return null; } };
      const incoming = listing('incoming');
      const scratch = listing('scratch');
      const leaked = all.filter(({ entry }) => entry.isFile()).flatMap(({ full }) => found(fs.readFileSync(full, 'utf8')));
      measured.batchFolder = { mode: octal(mode(folder)), files: all.filter(({ entry }) => entry.isFile()).map(({ full }) => path.relative(folder, full)), incoming, scratch };
      record('While Ready: the batch folder is 0700, its files 0600, incoming/ and scratch/ empty, no canary in it',
        mode(folder) === 0o700 && badDirs.length === 0 && badFiles.length === 0 && (incoming === null || incoming.length === 0) && (scratch === null || scratch.length === 0) && leaked.length === 0,
        `folder ${octal(mode(folder))}; other modes ${js([...badDirs, ...badFiles])}; incoming ${js(incoming)}; scratch ${js(scratch)}; files ${all.filter(({ entry }) => entry.isFile()).length}; canaries ${js(leaked)}`);
    } else {
      record('While Ready: the batch folder is 0700, its files 0600, incoming/ and scratch/ empty, no canary in it', false, `batch folders ${js(batchIds)}`);
    }
    const job = readyBatch ? await request('GET', `/api/import/uploads/${readyBatch}`) : null;
    const candidates = job?.body?.state?.batch?.candidates ?? [];
    await clickOn(page, q('[data-export-all]'));
    const allHere = await page.waitFor(`!!${q('[data-export-already]')}`, 60000);
    const status = await page.evaluate(`${q('[data-export-status]')}?.innerText.trim()`);
    await shoot(page, '2-export-all-imported-1280x800');
    // The threads persist in the background; wait for every one.
    const keyedThreads = async () => (await page.readStore('threads')).filter((thread) => thread.import_key);
    const stored = await until('every imported thread saved', async () => {
      const now = await keyedThreads();
      return now.length >= candidates.length ? now : null;
    }, 15000).catch(keyedThreads);
    imported = candidates.map((candidate) => ({ candidate, thread: stored.find((thread) => thread.import_key === `${candidate.source}:${candidate.id}`) }));
    const keyed = imported.filter((each) => each.thread).length;
    record('Upload ChatGPT parts and Grok: progress lines, batch lines, best effort Grok rows; Import all imports every row',
      progress.length >= 1 && /from ChatGPT and Grok\.$/.test(lines[0] ?? '') && grokRows.length >= 1 && grokRows.every((detail) => detail.endsWith(', best effort')) && chatgptRows.length >= 1 && chatgptRows.every((detail) => !detail.includes('best effort')) && titles.length === candidates.length && allHere && keyed === candidates.length && stored.length === candidates.length,
      `progress ${js(progress)}; lines ${js(lines)}; rows ${titles.length} (ChatGPT ${chatgptRows.length}, Grok ${grokRows.length}); candidates ${candidates.length}; all here ${allHere}; status ${js(status)}; threads with an import key ${stored.length}, matched ${keyed}`);

    // Closing discards the batch (F23).
    await closeDialogs(page);
    const gone = await until('the batch is discarded', () => batches().length === 0, 10000).catch(() => false);
    record('After the dialog closes, imports/ holds no batch', gone, `left ${js(batches())}`);
  });

  await check("Each imported thread opens with its transcript, and its first turn's request carries the transcript", async () => {
    if (imported.length === 0) throw new Error('nothing imported in the check before');
    const outcomes = [];
    for (const { candidate, thread } of imported) {
      if (!thread) {
        outcomes.push({ id: candidate.id, missing: true });
        continue;
      }
      // The transcript's first message text, as words (the page renders its Markdown).
      const firstLine = (thread.import_context ?? '').split('\n').map((line) => line.trim()).find((line) => line && !/^[#_-]/.test(line)) ?? '';
      const firstWords = firstLine.replace(/[*_`>\[\]]/g, '').split(/\s+/).slice(0, 4).join(' ');
      await page.evaluate(`location.hash = ${js(`#thread/${thread.id}`)}`);
      const shown = await page.waitFor(`!!document.querySelector('#chat-input') && (document.querySelector('main')?.innerText ?? '').includes(${js(firstWords)})`, 10000);
      const logText = await page.evaluate(`document.querySelector('main')?.innerText ?? ''`);
      const before = agentRequests.length;
      const typed = `Carry on ${tag}`;
      await page.evaluate(`document.querySelector('#chat-input').focus()`);
      await page.type(typed);
      await page.press('Enter');
      const sent = await until('the held turn', () => agentRequests.length > before, 10000).then(() => true).catch(() => false);
      const turn = agentRequests[before];
      const prompt = turn?.prompt ?? '';
      outcomes.push({
        id: candidate.id,
        source: candidate.source,
        shown,
        firstInLog: firstWords.length > 0 && logText.includes(firstWords),
        sent,
        carries: !!thread.import_context && prompt.startsWith(COPY.imported) && prompt.includes(thread.import_context.slice(0, 400)) && prompt.endsWith(typed),
        thread: turn?.thread_id === thread.id,
      });
      await page.waitFor(`!!document.querySelector('[aria-label="Send message"]')`, 5000);
    }
    await shoot(page, '2-imported-thread-1280x800');
    const bad = outcomes.filter((outcome) => outcome.missing || !outcome.shown || !outcome.firstInLog || !outcome.sent || !outcome.carries || !outcome.thread);
    record("Each imported thread opens with its transcript, and its first turn's request carries the transcript", bad.length === 0 && outcomes.length === imported.length,
      `threads ${outcomes.length}; bad ${js(bad)}`);
  });

  await check('Reopening the dialog with the same files shows ALREADY_HERE', async () => {
    await page.evaluate(`location.hash = '#/'`);
    await sleep(500);
    await openExportTab(page);
    await choose(page, [chatgptZip, grokJson]);
    const already = await page.waitFor(`${q('[data-export-already]')}?.innerText.trim() === ${js(COPY.alreadyHere)}`, 60000);
    await shoot(page, '2-already-here-1280x800');
    await closeDialogs(page);
    const gone = await until('the batch is discarded', () => batches().length === 0, 10000).catch(() => false);
    record('Reopening the dialog with the same files shows ALREADY_HERE', already && gone, `already ${already}; batch discarded ${gone}`);
  });

  // 3. Owner only.
  await check('Another loopback Origin gets 403 OWNER_ONLY_IMPORT on uploads, sessions, thread and agents', async () => {
    const outcomes = [];
    for (const origin of ['http://127.0.0.1:43092', 'http://localhost:5173']) {
      const boundary = `----bruhs${tag}`;
      const multipart = Buffer.from(`--${boundary}\r\nContent-Disposition: form-data; name="file"; filename="x.json"\r\nContent-Type: application/json\r\n\r\n${fs.readFileSync(grokJson, 'utf8')}\r\n--${boundary}--\r\n`);
      for (const [method, route] of [
        ['POST', '/api/import/uploads'],
        ['GET', '/api/import/sessions?source=claude_code&q=&limit=5'],
        ['GET', '/api/import/thread?source=claude_code&id=x'],
        ['GET', '/api/import/agents?source=openclaw'],
      ]) {
        const answer = method === 'POST' ? await rawPost(route, origin, multipart, `multipart/form-data; boundary=${boundary}`) : await request(method, route, { origin });
        outcomes.push({ origin, method, route: route.split('?')[0], status: answer.status, copy: answer.body?.error === COPY.ownerOnly, leaked: found(answer.text) });
      }
    }
    const leftover = batches();
    const bad = outcomes.filter((outcome) => outcome.status !== 403 || !outcome.copy || outcome.leaked.length > 0);
    record('Another loopback Origin gets 403 OWNER_ONLY_IMPORT on uploads, sessions, thread and agents', bad.length === 0 && leftover.length === 0,
      `answers ${js(outcomes.map((o) => `${o.origin} ${o.method} ${o.route} ${o.status}${o.copy ? '' : ' (other copy)'}`))}; batches made ${js(leftover)}`);
  });

  // 4. The hostile ZIP.
  await check('The hostile ZIP answers ARCHIVE_REFUSED; no escape file under the scratch home and no batch folder left', async () => {
    await openExportTab(page);
    await choose(page, [hostileZip]);
    const shown = await page.waitFor(`(${q('[data-export-error]')}?.innerText.trim() ?? '') !== ''`, 120000);
    const error = await page.evaluate(`${q('[data-export-error]')}?.innerText.trim()`);
    await shoot(page, '4-hostile-1280x800');
    await closeDialogs(page);
    // Each part on its own, through the API.
    const parts = {};
    for (const [name, file] of [['jsonBomb', bombZip], ['zipBomb', zipBombZip], ['nested3', nested3Zip], ['escape', escapeZip], ['nested2', nested2Zip]]) {
      const outcome = await uploadJob([file]);
      parts[name] = outcome.state === 'failed' ? outcome.message : outcome.state === 'ready' ? `ready: ${outcome.batch?.candidates?.length ?? 0} chats` : `${outcome.status} ${outcome.error}`;
    }
    const escapes = [home, outside, runDir].flatMap((dir) => walkFiles(dir)).filter(({ entry, full }) => /escape/i.test(entry.name) && !full.startsWith(files + path.sep)).map(({ full }) => path.relative(runDir, full));
    const left = await until('no batch folder', () => batches().length === 0, 10000).then(() => []).catch(() => batches());
    measured.hostileParts = parts;
    record('The hostile ZIP answers ARCHIVE_REFUSED; no escape file under the scratch home and no batch folder left',
      shown && error === COPY.archiveRefused && [COPY.archiveRefused, COPY.conversationTooLarge].includes(parts.jsonBomb) && parts.zipBomb === COPY.archiveRefused && parts.nested3 === COPY.archiveRefused && parts.escape === COPY.notAnExport && /^ready: 1 chats?$/.test(parts.nested2) && escapes.length === 0 && left.length === 0,
      `UI error ${js(error)}; parts ${js(parts)}; escape files ${js(escapes)}; batches left ${js(left)}`);
  });

  // 5. Claude.ai memory into By prompt. Import thread closes as Move in
  // opens, as on the desktop: nothing stacks under Move in, and the batch
  // goes with Import thread before any Escape (final review C-I1).
  const importDialog = `document.querySelector('[role=dialog][aria-label="Import thread"]')`;
  await check("The Claude ZIP shows memory_found_line; Use as a bruh's memory closes Import thread, discards its batch and opens By prompt prefilled", async () => {
    await openExportTab(page);
    await choose(page, [claudeZip]);
    const offered = await page.waitFor(`!!${q('[data-export-use-memory]')}`, 60000);
    const line = (await texts(page, '[data-export-memory]'))[0];
    await shoot(page, '5-claude-memory-1280x800');
    await clickOn(page, q('[data-export-use-memory]'));
    const byPrompt = await page.waitFor(`${moveTab('By prompt')}?.getAttribute('aria-selected') === 'true'`, 5000);
    const importClosed = await page.waitFor(`!${importDialog}`, 3000);
    const dialogs = await page.evaluate(`[...document.querySelectorAll('[role=dialog]')].map((d) => d.getAttribute('aria-label'))`);
    const discarded = await until('the batch is discarded with Import thread', () => batches().length === 0, 10000).then(() => true).catch(() => false);
    const prefilled = await page.evaluate(`${q('[data-move-in-answer]')}?.value ?? ''`);
    const focus = await page.waitFor(`document.activeElement?.hasAttribute('data-move-in-answer')`, 3000);
    await shoot(page, '5-by-prompt-prefilled-1280x800');
    // One Escape closes Move in and leaves no dialog behind it.
    await page.press('Escape');
    await sleep(300);
    const nothingLeft = await page.evaluate(`!document.querySelector('[role=dialog]')`);
    const landed = await page.evaluate(`(() => { const el = document.activeElement; return !!el && el !== document.body; })()`);
    await closeDialogs(page);
    record("The Claude ZIP shows memory_found_line; Use as a bruh's memory closes Import thread, discards its batch and opens By prompt prefilled",
      offered && line === COPY.memoryFound && byPrompt && importClosed && js(dialogs) === js(['Move in']) && discarded && /short answers/.test(prefilled) && /Bakes bread/.test(prefilled) && focus && nothingLeft && landed,
      `offered ${offered}; line ${js(line)}; By prompt ${byPrompt}; Import thread closed ${importClosed}; dialogs ${js(dialogs)}; batch discarded before Escape ${discarded}; prefilled ${js(prefilled.slice(0, 120))}; focus in the answer ${focus}; no dialog after one Escape ${nothingLeft}; focus kept ${landed}`);
  });

  // 7 and 8. Move in by keyboard, Make this bruh, the proposal.
  let pinch = null;
  await check('Move in by keyboard (⌘K, "Move in", Enter): OpenClaw lists ~/clawd with persona_lines and the skipped link line', async () => {
    await page.evaluate(`location.hash = '#/'`);
    await sleep(500);
    await palette(page, 'Move in');
    const selected = await page.waitFor(`${moveTab('OpenClaw')}?.getAttribute('aria-selected') === 'true'`, 5000);
    const listed = await page.waitFor(`[...document.querySelectorAll('[data-move-in-name]')].some((e) => e.innerText.trim() === 'Pinch')`, 15000);
    const folder = await page.evaluate(`[...document.querySelectorAll('[data-move-in-name]')].find((e) => e.innerText.trim() === 'Pinch')?.nextElementSibling?.innerText.trim()`);
    const places = (await texts(page, '[data-move-in-places]'))[0] ?? '';
    const lines = await texts(page, '[data-move-in-line]');
    const focus = await page.waitFor(`document.activeElement?.hasAttribute('data-move-in-folder')`, 3000);
    await shoot(page, '7-move-in-openclaw-1280x800');
    record('Move in by keyboard (⌘K, "Move in", Enter): OpenClaw lists ~/clawd with persona_lines and the skipped link line',
      selected && listed && [path.join(home, 'clawd'), '~/clawd'].includes(folder) && places.includes('~/clawd') && lines.includes(COPY.skippedLink) && lines.some((line) => line.startsWith('Instructions from ')) && lines.some((line) => line.startsWith('Memory from ')) && focus,
      `OpenClaw ${selected}; Pinch ${listed}; folder ${js(folder === path.join(home, 'clawd') ? '<home>/clawd' : folder)}; ${js(places)}; lines ${js(lines)}; focus in Folder ${focus}`);
  });

  await check('Make this bruh opens Create prefilled; Create makes a Read only bruh with empty memory and one pending SetMemory proposal; Decline leaves it empty', async () => {
    await clickOn(page, q('[data-move-in-make]'));
    const form = await page.waitFor(`!!document.querySelector('form[aria-label="New bruh"]') && !document.querySelector('[role=tablist][aria-label="Move in"]')`, 5000);
    const name = await page.evaluate(`document.querySelector('form[aria-label="New bruh"] input[aria-label=Name]')?.value`);
    const job = await page.evaluate(`document.querySelector('form[aria-label="New bruh"] textarea[aria-label=Job]')?.value ?? ''`);
    await shoot(page, '8-create-prefilled-1280x800');
    await clickOn(page, `document.querySelector('form[aria-label="New bruh"] button[type=submit]')`);
    const opened = await page.waitFor(`document.querySelector('#bruh-title')?.innerText === 'Pinch'`, 15000);
    const snap = await until('the proposal', async () => {
      const now = await snapshot();
      const agent = now.agents.find((a) => a.name === 'Pinch');
      return agent && now.proposals.some((p) => p.status === 'pending' && p.change.type === 'set_memory' && p.change.agent_id === agent.id) ? now : null;
    }, 15000);
    const agent = snap.agents.find((a) => a.name === 'Pinch');
    const bruh = snap.bruhs.find((b) => b.name === 'Pinch');
    pinch = agent;
    const proposals = snap.proposals.filter((p) => p.status === 'pending' && p.change.type === 'set_memory' && p.change.agent_id === agent.id);
    await clickOn(page, `[...document.querySelectorAll('[role=tab]')].find((t) => t.innerText.trim() === 'Memory')`);
    const card = await page.waitFor(`!!${q('[data-proposed-card]')}`, 5000);
    const shown = await page.evaluate(`${q('[data-proposed-text]')}?.innerText ?? ''`);
    await shoot(page, '8-proposed-memory-1280x800');
    await clickOn(page, q('[data-proposed-decline]'));
    const removed = await page.waitFor(`!${q('[data-proposed-card]')}`, 10000);
    const after = await snapshot();
    const focusAfter = await page.evaluate(`document.activeElement && document.activeElement !== document.body`);
    record('Make this bruh opens Create prefilled; Create makes a Read only bruh with empty memory and one pending SetMemory proposal; Decline leaves it empty',
      form && name === 'Pinch' && /short sentences/.test(job) && opened && agent.memory === '' && bruh?.access?.level === 'read_only' && proposals.length === 1 && card && shown.trim() === proposals[0].change.memory.trim() && /Lisbon ships/.test(shown) && removed && after.agents.find((a) => a.id === agent.id)?.memory === '' && focusAfter,
      `form ${form}; name ${js(name)}; job ${js(job.slice(0, 60))}; page ${opened}; memory at create ${js(agent.memory)}; access ${bruh?.access?.level}; proposals ${proposals.length}; card ${card}; text matches ${shown.trim() === proposals[0]?.change.memory.trim()}; declined ${removed}; memory after ${js(after.agents.find((a) => a.id === agent.id)?.memory)}; focus kept ${focusAfter}`);
  });

  await check('When the opener is gone (Make this bruh, then Escape on Create), focus returns to the rail\'s New bruh or the Move in dialog, never the body', async () => {
    await palette(page, 'Move in');
    await page.waitFor(`!!${q('[data-move-in-make]')}`, 15000);
    await clickOn(page, q('[data-move-in-make]'));
    await page.waitFor(`!!document.querySelector('form[aria-label="New bruh"]')`, 5000);
    await page.press('Escape');
    await sleep(300);
    const landed = await page.evaluate(`(() => { const el = document.activeElement; return { body: !el || el === document.body, crewNew: !!el?.hasAttribute('data-crew-new'), inMoveIn: !!el?.closest('[role=dialog]')?.querySelector('[role=tablist][aria-label="Move in"]'), html: el?.outerHTML?.slice(0, 80) ?? null }; })()`);
    await closeDialogs(page);
    record('When the opener is gone (Make this bruh, then Escape on Create), focus returns to the rail\'s New bruh or the Move in dialog, never the body',
      !landed.body && (landed.crewNew || landed.inMoveIn), js(landed));
  });

  await check('A second import of the same persona, then Approve memory, sets it', async () => {
    await palette(page, 'Move in');
    await page.waitFor(`!!${q('[data-move-in-make]')}`, 15000);
    await clickOn(page, q('[data-move-in-make]'));
    await page.waitFor(`!!document.querySelector('form[aria-label="New bruh"]')`, 5000);
    const before = new Set((await snapshot()).agents.map((a) => a.id));
    await clickOn(page, `document.querySelector('form[aria-label="New bruh"] button[type=submit]')`);
    const snap = await until('the second proposal', async () => {
      const now = await snapshot();
      const agent = now.agents.find((a) => !before.has(a.id) && a.name.startsWith('Pinch'));
      return agent && now.proposals.some((p) => p.status === 'pending' && p.change.type === 'set_memory' && p.change.agent_id === agent.id) ? { now, agent } : null;
    }, 15000);
    const proposal = snap.now.proposals.find((p) => p.status === 'pending' && p.change.agent_id === snap.agent.id);
    await page.waitFor(`!!document.querySelector('#bruh-title')`, 10000);
    await clickOn(page, `[...document.querySelectorAll('[role=tab]')].find((t) => t.innerText.trim() === 'Memory')`);
    await page.waitFor(`!!${q('[data-proposed-approve]')}`, 5000);
    await clickOn(page, q('[data-proposed-approve]'));
    const removed = await page.waitFor(`!${q('[data-proposed-card]')}`, 10000);
    const memory = (await snapshot()).agents.find((a) => a.id === snap.agent.id)?.memory;
    await shoot(page, '8-memory-approved-1280x800');
    record('A second import of the same persona, then Approve memory, sets it', removed && memory === proposal.change.memory && /Lisbon ships/.test(memory ?? ''),
      `card gone ${removed}; memory set ${memory === proposal.change.memory}; name ${js(snap.agent.name)}`);
  });

  // 10. By prompt for Scout.
  await check('By prompt: Copy prompt puts MOVE_IN_PROMPT on the clipboard; an answer for Scout shows memory_proposed_line; Cmd+Enter proposes; a direct SetMemory shows MEMORY_CHANGED', async () => {
    await palette(page, 'Move in');
    await page.waitFor(`!!(${moveTab('By prompt')})`, 5000);
    await clickOn(page, moveTab('By prompt'));
    await page.waitFor(`!!${q('[data-move-in-bruh]')}`, 10000);
    await page.evaluate(`(() => { const s = ${q('[data-move-in-bruh]')}; s.value = ${js(scoutId)}; s.dispatchEvent(new Event('change', { bubbles: true })); })()`);
    await clickOn(page, q('[data-move-in-copy]'));
    const copied = await page.waitFor(`${q('[data-move-in-copy]')}.innerText.trim() === 'Copied'`, 2000);
    const clip = await page.send('Runtime.evaluate', { expression: 'navigator.clipboard.readText()', awaitPromise: true, returnByValue: true, userGesture: true });
    const clipboard = clip.result?.value ?? `error: ${clip.exceptionDetails?.exception?.description ?? ''}`;
    await clickOn(page, q('[data-move-in-answer]'));
    await page.type(`Likes a walk at lunch.\nUses a paper planner ${tag}.`);
    await clickOn(page, q('[data-move-in-propose]'));
    const status = await page.waitFor(`${q('[data-move-in-status]')}?.innerText.trim() === ${js(COPY.proposedScout)}`, 10000);
    const statusText = await page.evaluate(`${q('[data-move-in-status]')}?.innerText.trim()`);
    await shoot(page, '10-by-prompt-proposed-1280x800');
    // Cmd+Enter in Their answer proposes too, as on the desktop (C-I3).
    const pendingFor = async () => (await snapshot()).proposals.filter((p) => p.status === 'pending' && p.change.type === 'set_memory' && p.change.agent_id === scoutId).length;
    const pendingBefore = await pendingFor();
    await clickOn(page, q('[data-move-in-answer]'));
    await page.type(`Keeps a garden ${tag}.`);
    for (const type of ['keyDown', 'keyUp']) await page.send('Input.dispatchKeyEvent', { type, key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, modifiers: 4 });
    const shortcut = await until('the Cmd+Enter proposal', async () => ((await pendingFor()) === pendingBefore + 1 ? true : null), 10000).catch(() => false);
    const cleared = await page.waitFor(`${q('[data-move-in-answer]')}?.value === ''`, 5000);
    await closeDialogs(page);
    await command({ action: 'apply', change: { type: 'set_memory', agent_id: scoutId, memory: `Rewritten by the owner ${tag}.` } });
    // Another client changed it: open Scout from the rail so the page reads the crew afresh.
    await page.evaluate(`location.hash = '#/'`);
    await sleep(600);
    if (!(await page.evaluate(`!!document.querySelector('[data-crew-rail]')`))) {
      await palette(page, 'Toggle sidebar');
      await page.waitFor(`!!document.querySelector('[data-crew-rail]')`, 3000);
    }
    await clickOn(page, `[...document.querySelectorAll('[data-crew-rail] button')].find((b) => /Scout/.test(b.innerText))`);
    await page.waitFor(`document.querySelector('#bruh-title')?.innerText === 'Scout'`, 5000);
    await clickOn(page, `[...document.querySelectorAll('[role=tab]')].find((t) => t.innerText.trim() === 'Memory')`);
    const stale = await page.waitFor(`${q('[data-proposed-stale]')}?.innerText.trim() === ${js(COPY.memoryChanged)}`, 10000);
    await shoot(page, '10-proposed-stale-1280x800');
    record('By prompt: Copy prompt puts MOVE_IN_PROMPT on the clipboard; an answer for Scout shows memory_proposed_line; Cmd+Enter proposes; a direct SetMemory shows MEMORY_CHANGED',
      copied && clipboard === COPY.movePrompt && status && shortcut && cleared && stale,
      `Copied ${copied}; clipboard ${clipboard === COPY.movePrompt ? 'MOVE_IN_PROMPT' : js(String(clipboard).slice(0, 80))}; status ${js(statusText)}; Cmd+Enter proposed ${shortcut} (pending before ${pendingBefore}), answer cleared ${cleared}; stale warning ${stale}`);
  });

  // 11. Hermes.
  await check('The Hermes tab lists ~/.hermes with its § entries', async () => {
    await palette(page, 'Move in');
    await page.waitFor(`!!(${moveTab('Hermes Agent')})`, 5000);
    await clickOn(page, moveTab('Hermes Agent'));
    const listed = await page.waitFor(`document.querySelectorAll('[data-move-in-name]').length === 1`, 15000);
    const folder = await page.evaluate(`document.querySelector('[data-move-in-name]')?.nextElementSibling?.innerText.trim()`);
    const lines = await texts(page, '[data-move-in-line]');
    const places = (await texts(page, '[data-move-in-places]'))[0] ?? '';
    await shoot(page, '11-move-in-hermes-1280x800');
    const scan = await api('GET', '/api/import/agents?source=hermes');
    const persona = scan.personas[0] ?? {};
    const memory = persona.memory ?? '';
    const entries = hermesEntries.map((entry) => memory.split(entry).length - 1);
    await closeDialogs(page);
    record('The Hermes tab lists ~/.hermes with its § entries', listed && [path.join(home, '.hermes'), '~/.hermes'].includes(folder) && places.includes('~/.hermes') && lines.some((line) => /^Memory from .*USER\.md/.test(line) && /MEMORY\.md/.test(line)) && entries.every((count) => count === 1) && !memory.includes('§'),
      `listed ${listed}; folder ${js(folder === path.join(home, '.hermes') ? '<home>/.hermes' : folder)}; ${js(places)}; lines ${js(lines)}; each entry once ${js(entries)}; section signs left ${memory.includes('§')}`);
  });

  // 9. Canaries.
  await check('No canary in any /api/import answer (agents included), the page or the server log', async () => {
    await sleep(1000);
    const importBodies = bodies.filter((entry) => entry.url.startsWith('/api/import/'));
    const agentBodies = importBodies.filter((entry) => entry.url.startsWith('/api/import/agents'));
    const leaks = importBodies.flatMap((entry) => found(entry.body).map((name) => `${entry.url}:${name}`));
    const logText = fs.readFileSync(serverLog, 'utf8');
    const pageText = await page.evaluate('document.body.innerText');
    record('No canary in any /api/import answer (agents included), the page or the server log', leaks.length === 0 && found(logText).length === 0 && found(pageText).length === 0 && agentBodies.length >= 3,
      `answers read ${importBodies.length} (agents ${agentBodies.length}); leaks ${js(leaks)}; log ${js(found(logText))}; page ${js(found(pageText))}`);
  });

  // The desktop's minimum window size, for the side-by-side captures.
  await check('640×480: the Export file tab, Move in and By prompt have no horizontal scroll', async () => {
    const small = await open({ width: 640, height: 480 });
    await small.goto(`${base}/#/`);
    await small.appReady();
    await sleep(1500);
    await openExportTab(small);
    const exportLayout = await layoutOf(small, `document.querySelector('[role=dialog]')`);
    await shoot(small, '13-export-start-640x480');
    await choose(small, [chatgptZip, grokJson]);
    const ready = await small.waitFor(`!!${q('[data-export-all]')} || !!${q('[data-export-already]')}`, 60000);
    const readyLayout = await layoutOf(small, `document.querySelector('[role=dialog]')`);
    await shoot(small, '13-export-ready-640x480');
    await closeDialogs(small);
    await palette(small, 'Move in');
    await small.waitFor(`!!${q('[data-move-in-make]')}`, 15000);
    const moveLayout = await layoutOf(small, `document.querySelector('[data-move-in-agents]').closest('[role=dialog]')`);
    await shoot(small, '13-move-in-640x480');
    await clickOn(small, moveTab('By prompt'));
    await small.waitFor(`!!${q('[data-move-in-bruh]')}`, 10000);
    const promptLayout = await layoutOf(small, `document.querySelector('[data-move-in-by-prompt]').closest('[role=dialog]')`);
    await shoot(small, '13-move-in-prompt-640x480');
    await closeDialogs(small);
    await small.close();
    const gone = await until('the batch is discarded', () => batches().length === 0, 10000).catch(() => false);
    const layouts = [exportLayout, readyLayout, moveLayout, promptLayout];
    record('640×480: the Export file tab, Move in and By prompt have no horizontal scroll', ready && gone && layouts.every((layout) => layout.sideways <= 0 && layout.dialogSideways <= 0),
      `ready ${ready}; batch discarded ${gone}; sideways ${layouts.map((layout) => `${layout.sideways}/${layout.dialogSideways}`).join(' ')}`);
  });

  // 12. Phone, reduced motion, panics.
  await check('390×844 touch: the dialogs\' controls (the ready batch\'s included) are at least 44px with no horizontal page scroll; reduced motion draws no animation frames', async () => {
    const phone = await open({ width: 390, height: 844, touch: true });
    await phone.goto(`${base}/#/`);
    await phone.appReady();
    await sleep(1500);
    await openExportTab(phone);
    const exportLayout = await layoutOf(phone, `document.querySelector('[role=dialog]')`);
    await shoot(phone, '12-export-390x844');
    // The ready batch too: the search, the rows, Use as a bruh's memory,
    // Import all and Choose another (the Claude chats aren't imported yet).
    await choose(phone, [claudeZip]);
    const readyShown = await phone.waitFor(`!!${q('[data-export-search]')} && !!${q('[data-export-all]')}`, 60000);
    const readyLayout = await layoutOf(phone, `document.querySelector('[role=dialog]')`);
    const searchHeight = await phone.evaluate(`Math.round(${q('[data-export-search]')}?.getBoundingClientRect().height ?? 0)`);
    await shoot(phone, '12-export-ready-390x844');
    await closeDialogs(phone);
    const readyGone = await until('the batch is discarded', () => batches().length === 0, 10000).then(() => true).catch(() => false);
    await palette(phone, 'Move in');
    await phone.waitFor(`!!${q('[data-move-in-make]')}`, 15000);
    const moveLayout = await layoutOf(phone, `document.querySelector('[data-move-in-agents]').closest('[role=dialog]')`);
    await shoot(phone, '12-move-in-390x844');
    await clickOn(phone, moveTab('By prompt'));
    await phone.waitFor(`!!${q('[data-move-in-bruh]')}`, 10000);
    const promptLayout = await layoutOf(phone, `document.querySelector('[data-move-in-by-prompt]').closest('[role=dialog]')`);
    await shoot(phone, '12-by-prompt-390x844');
    await closeDialogs(phone);
    await phone.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await openExportTab(phone);
    await sleep(800);
    await phone.evaluate('window.__frames = 0');
    await sleep(1000);
    const framesOpen = await phone.evaluate('window.__frames');
    await clickOn(phone, importTab('Codex'));
    await sleep(600);
    await phone.evaluate('window.__frames = 0');
    await sleep(1000);
    const framesTab = await phone.evaluate('window.__frames');
    await closeDialogs(phone);
    await palette(phone, 'Move in');
    await phone.waitFor(`!!${q('[data-move-in-make]')}`, 15000);
    await sleep(800);
    await phone.evaluate('window.__frames = 0');
    await sleep(1000);
    const framesMove = await phone.evaluate('window.__frames');
    await closeDialogs(phone);
    await phone.close();
    const layouts = [exportLayout, readyLayout, moveLayout, promptLayout];
    const small = layouts.flatMap((layout) => layout.controls.filter((control) => control.height < 44));
    measured.touch = { export: exportLayout.controls.length, ready: readyLayout.controls.length, search: searchHeight, moveIn: moveLayout.controls.length, byPrompt: promptLayout.controls.length };
    record('390×844 touch: the dialogs\' controls (the ready batch\'s included) are at least 44px with no horizontal page scroll; reduced motion draws no animation frames',
      exportLayout.coarse && readyShown && readyGone && searchHeight >= 44 && small.length === 0 && layouts.every((layout) => layout.sideways <= 0 && layout.controls.length > 0) && framesOpen === 0 && framesTab === 0 && framesMove === 0,
      `controls ${js(measured.touch)}; ready ${readyShown}; batch discarded ${readyGone}; under 44px ${js(small)}; sideways ${layouts.map((l) => l.sideways).join('/')}; frames open ${framesOpen}, after a tab ${framesTab}, Move in ${framesMove}`);
  });

  const pageErrors = pages.flatMap((each) => each.runtimeErrors.filter(Boolean).map((error) => String(error).slice(0, 200)));
  record('No panics in the page log', panics.length === 0, `${panics.join(' | ')}${pageErrors.length ? `; other page errors: ${pageErrors.slice(0, 5).join(' | ')}` : ''}`);
  exitCode = results.every((result) => result.pass) ? 0 : 1;
} catch (error) {
  record('the run', false, error?.stack ?? String(error));
} finally {
  const failed = results.filter((result) => !result.pass).length;
  fs.writeFileSync(path.join(runDir, 'results.json'), sanitize(js({ when: new Date().toISOString(), results, measured }, null, 2)));
  await cleanup();
  console.log(`Measured: ${sanitize(js(measured))}`);
  console.log(`Screenshots: ${shots}`);
  console.log(`${results.length - failed}/${results.length} checks passed.`);
  console.log(exitCode === 0 ? 'All import checks passed.' : `${failed} import check(s) failed.`);
}
process.exit(exitCode);
