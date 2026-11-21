// Acceptance run for backup on the web (docs/backup.md; PARITY-21 in
// docs/web-desktop-parity-goal-2026-11-15.md): two scratch homes with
// different paths, each with its own bruhs-server, a backup made on A by
// keyboard and restored onto B, then put back and deleted. It covers the
// file (magic, nothing in clear), the owner-only routes, leaks on disk, the
// preview and its path rewrite, the server's exit and "start Bruhs again"
// (the script restarts a server that exited), the restored data on B (bruh,
// work folder, room, mail row, vault values and their re-keyed accounts,
// audio), the browser part (IndexedDB, localStorage, a Blob byte for byte and
// the values JSON can't carry), other tabs during a browser restore (an
// unsent draft, a back and forward cache entry), a crafted archive with a
// `..` entry, phone-size touch, reduced motion and a 100 MB Blob with the
// tab's peak memory on dump and restore. Every check prints ok or FAIL; the
// exit status is 0 only when all pass.
//
//   cargo build -p bruhs-server
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43081 trunk build --dist "$TMPDIR/bruhs-backup-dist-a")
//   (cd bruhs-web && BRUHS_API_URL=http://127.0.0.1:43082 trunk build --dist "$TMPDIR/bruhs-backup-dist-b")
//   node scripts/verify-backup.mjs "$PWD" "$TMPDIR/bruhs-backup-dist-a" "$TMPDIR/bruhs-backup-dist-b"
//
// Each server is the debug target/debug/bruhs-server with a scratch HOME,
// BRUHS_SECRET_STORE=file:<home>/keys (D26: keychain items in
// <home>/keys/items.json, so the owner's keychain is never touched and the
// restore's own process sees the same items), no BRUHS_* data path
// overrides, a capture URL nothing listens on and BRUHS_APPS_UPSTREAM naming
// a stand-in GitHub this script runs on loopback (for the app token). A on
// port 43081, B on 43082. Headless Chrome (BRUHS_VERIFY_CHROME, default the
// macOS app) runs on BRUHS_VERIFY_CDP_PORT (default 9481) with its own
// profile and download folder. Results, logs and screenshots go to
// $TMPDIR/bruhs-backup-verify/, cleared on each run. Canaries are random;
// nothing this script prints or writes holds one.
//
// BRUHS_VERIFY_SKIP_LARGE=1 leaves out the 100 MB Blob check (and fails the
// run, as a partial run never passes).
import { spawn, spawnSync } from 'node:child_process';
import crypto, { randomBytes } from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { launchChrome, newContext, newPage, sleep } from '../bruhs-web/tests/cdp_harness.mjs';

const root = path.resolve(process.argv[2] ?? '');
const dists = { a: path.resolve(process.argv[3] ?? ''), b: path.resolve(process.argv[4] ?? '') };
const serverBin = path.join(root, 'target/debug/bruhs-server');
for (const [label, file] of [['bruhs-server', serverBin], ['web bundle for A', path.join(dists.a, 'index.html')], ['web bundle for B', path.join(dists.b, 'index.html')]]) {
  if (!fs.existsSync(file)) {
    console.error(`${label} not found at ${file}; see the usage at the top of this script.`);
    process.exit(2);
  }
}
if (process.platform !== 'darwin' && process.platform !== 'linux') {
  console.error('Backup runs on macOS and Linux only.');
  process.exit(2);
}
const cdpPort = Number(process.env.BRUHS_VERIFY_CDP_PORT ?? 9481);
const runDir = path.join(fs.realpathSync(tmpdir()), 'bruhs-backup-verify');
fs.rmSync(runDir, { recursive: true, force: true });
const shots = path.join(runDir, 'shots');
const downloads = path.join(runDir, 'downloads');
for (const dir of [shots, downloads]) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
const runStart = Date.now();
console.log(`Run directory: ${runDir}`);
const js = JSON.stringify;

// ── Canaries ─────────────────────────────────────────────────────────────────
const hex = (bytes = 12) => randomBytes(bytes).toString('hex');
const tag = hex(3);
const keychain = { global: `gv-${hex(16)}`, project: `pv-${hex(16)}`, app: `ghp_${hex(18)}` };
const password = `pw ${hex(10)} ok`;
const wrongPassword = `nope ${hex(8)} no`;
const threadText = `thread-${hex(10)}`;
const workText = `work-${hex(10)}`;
const keychainSet = Object.entries(keychain);
const secretSet = [...keychainSet, ['password', password], ['wrongPassword', wrongPassword]];
const found = (text, set = secretSet) => set.filter(([, value]) => String(text ?? '').includes(value)).map(([name]) => name);
const sanitize = (text) => [...secretSet, ['threadText', threadText], ['workText', workText]].reduce((out, [name, value]) => out.replaceAll(value, `[${name}]`), String(text ?? ''));
const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');

// ── Homes and servers ────────────────────────────────────────────────────────
const rootsOf = (home) => process.platform === 'darwin'
  ? { core: path.join(home, 'Library/Application Support/dev.bruhs.bruhs-ai'), desktop: path.join(home, 'Library/Application Support/bruhs-ai') }
  : { core: path.join(home, '.local/share/bruhs-ai'), desktop: path.join(home, '.local/share/bruhs-ai') };
const servers = {};
for (const [name, dir, port] of [['a', 'home-a', 43081], ['b', 'home-b-other', 43082]]) {
  const home = path.join(runDir, dir);
  fs.mkdirSync(path.join(home, 'tmp'), { recursive: true, mode: 0o700 });
  servers[name] = { name, home, port, base: `http://127.0.0.1:${port}`, dist: dists[name], keys: path.join(home, 'keys'), log: path.join(runDir, `server-${name}.log`), ...rootsOf(home), proc: null, exited: null, starts: 0 };
}
const A = servers.a;
const B = servers.b;
const projectA = path.join(A.home, 'src/app');
const projectB = path.join(B.home, 'src/app');
fs.mkdirSync(projectA, { recursive: true });
fs.mkdirSync(projectB, { recursive: true });

// A stand-in GitHub for the app token: GET /user names the account.
const upstream = http.createServer((request, response) => {
  request.resume();
  request.on('end', () => {
    const host = request.headers['x-bruhs-app-host'] ?? '';
    const ok = request.method === 'GET' && host === 'api.github.com' && request.url.startsWith('/user');
    const text = js(ok ? { login: `octo-${tag}`, id: 7 } : { message: 'Not Found' });
    response.writeHead(ok ? 200 : 404, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(text) });
    response.end(text);
  });
});
await new Promise((resolve) => upstream.listen(0, '127.0.0.1', resolve));
const upstreamUrl = `http://127.0.0.1:${upstream.address().port}`;

const portBusy = async (port) => {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
    return true;
  } catch (error) {
    return error?.cause?.code !== 'ECONNREFUSED';
  }
};
for (const port of [A.port, B.port, A.port + 2, cdpPort]) {
  if (await portBusy(port)) {
    console.error(`Port ${port} is in use; free it or set BRUHS_VERIFY_CDP_PORT.`);
    upstream.close();
    process.exit(2);
  }
}

const children = new Set();
async function startServer(server) {
  const log = fs.openSync(server.log, 'a');
  fs.writeSync(log, `\n==== start ${server.starts + 1} at ${new Date().toISOString()}\n`);
  const child = spawn(serverBin, [], {
    env: {
      PATH: '/usr/bin:/bin:/usr/sbin:/sbin',
      HOME: server.homeEnv ?? server.home,
      TMPDIR: path.join(server.home, 'tmp'),
      BRUHS_SERVER_PORT: String(server.port),
      BRUHS_WEB_DIST: server.dist,
      BRUHS_CAPTURE_URL: 'http://127.0.0.1:9',
      BRUHS_SECRET_STORE: `file:${server.keys}`,
      BRUHS_APPS_UPSTREAM: upstreamUrl,
      RUST_LOG: 'info',
    },
    stdio: ['ignore', log, log],
  });
  fs.closeSync(log);
  server.starts += 1;
  server.proc = child;
  server.exited = new Promise((resolve) => child.once('exit', (code, signal) => resolve({ code, signal, at: Date.now() })));
  children.add(child);
  const started = Date.now();
  for (;;) {
    try {
      if ((await fetch(`${server.base}/api/health`, { signal: AbortSignal.timeout(1000) })).ok) break;
    } catch {}
    if (child.exitCode !== null) throw new Error(`bruhs-server ${server.name} exited at start (code ${child.exitCode}); see ${path.basename(server.log)}`);
    if (Date.now() - started > 120000) throw new Error(`bruhs-server ${server.name} did not answer in 2 minutes`);
    await sleep(200);
  }
  return Date.now() - started;
}
const descendants = (pids) => {
  const table = spawnSync('/bin/ps', ['-axo', 'pid=,ppid='], { encoding: 'utf8' }).stdout.trim().split('\n').map((line) => line.trim().split(/\s+/).map(Number));
  const seen = new Set();
  let frontier = pids.filter(Boolean);
  while (frontier.length > 0) {
    const next = table.filter(([, ppid]) => frontier.includes(ppid)).map(([pid]) => pid).filter((pid) => !seen.has(pid));
    for (const pid of next) seen.add(pid);
    frontier = next;
  }
  return [...seen];
};
// The detached appliers this run's servers started (`<serverBin> restore-apply`).
const appliers = () => spawnSync('/bin/ps', ['-axo', 'pid=,command='], { encoding: 'utf8' }).stdout.split('\n')
  .map((line) => line.trim()).filter((line) => line.includes(`${serverBin} restore-apply`)).map((line) => Number(line.split(/\s+/)[0]));
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
  const below = descendants([...children].map((child) => child.pid));
  for (const child of [...children].reverse()) await stopChild(child);
  for (const pid of [...below, ...appliers()]) try { process.kill(pid, 'SIGKILL'); } catch {}
  await new Promise((resolve) => upstream.close(resolve));
}
process.on('SIGINT', () => cleanup().then(() => process.exit(130)));
// An error outside a check (the DevTools connection) still cleans up.
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

// ── Files ────────────────────────────────────────────────────────────────────
const filesUnder = (dir, { since = 0, skip = () => false, maxBytes = 512 << 20 } = {}) => {
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
      if (skip(full)) continue;
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        try {
          const info = fs.statSync(full);
          if (info.mtimeMs >= since && info.size <= maxBytes) out.push(full);
        } catch {}
      }
    }
  };
  walk(dir);
  return out;
};
const fileLeaks = (files, set = secretSet) => files.flatMap((file) => {
  let bytes;
  try {
    bytes = fs.readFileSync(file);
  } catch {
    return [];
  }
  return set.filter(([, value]) => bytes.includes(Buffer.from(value))).map(([name]) => `${file}:${name}`);
});
const items = (server) => {
  try {
    return JSON.parse(fs.readFileSync(path.join(server.keys, 'items.json'), 'utf8'));
  } catch {
    return {};
  }
};
const findDirs = (dir, test) => {
  const out = [];
  const walk = (folder, depth) => {
    if (depth > 12) return;
    let entries = [];
    try {
      entries = fs.readdirSync(folder, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(folder, entry.name);
      if (test(full, entry)) out.push(full);
      if (entry.isDirectory()) walk(full, depth + 1);
    }
  };
  walk(dir, 0);
  return out;
};
const sqlite = (file, sql) => spawnSync('/usr/bin/sqlite3', ['-readonly', file, sql], { encoding: 'utf8' });

// ── HTTP as the owner (no Origin is the native client; the page's own Origin here) ──
const bodies = [];
const request = async (server, method, route, body, { origin = server.base, raw = null, type = null } = {}) => {
  const headers = { ...(origin ? { origin } : {}) };
  if (body) headers['content-type'] = 'application/json';
  if (raw) headers['content-type'] = type ?? 'application/octet-stream';
  const response = await fetch(`${server.base}${route}`, { method, headers, body: raw ?? (body ? js(body) : undefined) });
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
const api = async (server, method, route, body) => {
  const answer = await request(server, method, route, body);
  if (answer.status < 200 || answer.status > 299) throw new Error(`${server.name} ${method} ${route}: ${answer.status} ${answer.text.slice(0, 300)}`);
  return answer.body;
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
  passwordsDiffer: "The passwords don't match.",
  webIncluded: "This browser's data goes in too.",
  wrongPassword: "That password doesn't open this backup, or the file was changed.",
  unsafeEntry: "This backup holds a file Bruhs won't write. Nothing was changed.",
  ownerOnly: 'Open Bruhs from its own address to make or restore a backup.',
  stopping: "Bruhs is stopping to finish the restore. This page reconnects when it's back.",
  restored: 'Restored. Your bruhs are paused: resume them on their pages.',
  quitDesktop: 'Quit the Bruhs desktop app first, or restore from the desktop app.',
  skipped: 'Some browser data comes from a newer Bruhs, so it was left out: whiteboard_assets.',
  webIncludedPreview: 'Browser data: included.',
  locking: "Locking this browser's data",
};

// ── Renderer memory ──────────────────────────────────────────────────────────
// The largest Chrome renderer's resident size, sampled every 100 ms while
// `body` runs: the tab doing the work is the largest.
async function peakRenderer(body) {
  const sample = () => {
    if (!chrome?.pid) return 0;
    const below = new Set(descendants([chrome.pid]));
    const rows = spawnSync('/bin/ps', ['-axo', 'pid=,rss=,command='], { encoding: 'utf8' }).stdout.split('\n').map((line) => line.trim().match(/^(\d+)\s+(\d+)\s+(.*)$/)).filter(Boolean);
    return Math.max(0, ...rows.filter(([, pid, , command]) => below.has(Number(pid)) && command.includes('--type=renderer')).map(([, , rss]) => Number(rss) * 1024));
  };
  const baseline = sample();
  let peak = baseline;
  let running = true;
  const sampler = (async () => {
    while (running) {
      peak = Math.max(peak, sample());
      await sleep(100);
    }
  })();
  try {
    await body();
  } finally {
    running = false;
    await sampler;
  }
  return { baselineMiB: Math.round(baseline / 1048576), peakMiB: Math.round(peak / 1048576), extraMiB: Math.round((peak - baseline) / 1048576) };
}

// ── A crafted archive (same format, Node crypto) ─────────────────────────────
const tarEntry = (name, body) => {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, 'utf8');
  header.write('0000644\0', 100);
  header.write('0000000\0', 108);
  header.write('0000000\0', 116);
  header.write(`${body.length.toString(8).padStart(11, '0')}\0`, 124);
  header.write(`${Math.floor(Date.now() / 1000).toString(8).padStart(11, '0')}\0`, 136);
  header.write('        ', 148);
  header.write('0', 156);
  header.write('ustar\0', 257);
  header.write('00', 263);
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, '0')}\0 `, 148);
  const padding = Buffer.alloc((512 - (body.length % 512)) % 512);
  return Buffer.concat([header, body, padding]);
};
function craftArchive(secret, entries) {
  const FLOOR = { memory: 19456, passes: 2, lanes: 1 };
  const chunk = 1 << 20;
  const header = Buffer.alloc(64);
  header.write('BRUHSBAK', 0, 'latin1');
  header.writeUInt16BE(1, 8);
  header[10] = 1; // a full backup
  header[11] = 1; // Argon2id
  header.writeUInt32BE(FLOOR.memory, 12);
  header.writeUInt32BE(FLOOR.passes, 16);
  header.writeUInt32BE(FLOOR.lanes, 20);
  const salt = randomBytes(16);
  const prefix = randomBytes(7);
  salt.copy(header, 24);
  prefix.copy(header, 40);
  header.writeUInt32BE(chunk, 48);
  const key = crypto.argon2Sync('argon2id', { message: Buffer.from(secret.normalize('NFC')), nonce: salt, parallelism: FLOOR.lanes, tagLength: 32, memory: FLOOR.memory, passes: FLOOR.passes });
  const plain = Buffer.concat([...entries.map(([name, body]) => tarEntry(name, Buffer.from(body))), Buffer.alloc(1024)]);
  const parts = [header];
  const count = Math.max(1, Math.ceil(plain.length / chunk));
  for (let index = 0; index < count; index += 1) {
    const nonce = Buffer.alloc(12);
    prefix.copy(nonce, 0);
    nonce.writeUInt32BE(index, 7);
    nonce[11] = index === count - 1 ? 1 : 0;
    const cipher = crypto.createCipheriv('aes-256-gcm', key, nonce);
    cipher.setAAD(header);
    parts.push(cipher.update(plain.subarray(index * chunk, (index + 1) * chunk)), cipher.final(), cipher.getAuthTag());
  }
  return Buffer.concat(parts);
}

// ── In-page helpers ──────────────────────────────────────────────────────────
// Writes `records` ({ store: [record] }) through a connection of its own;
// a record's `$blob` field (base64) becomes a Blob, `$cases` the values JSON
// can't carry, `$large` a Blob of that many random MiB (its SHA-256 goes in
// window.__largeSha).
const putRecords = (records) => `(async () => {
  const records = ${js(records)};
  const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const hexOf = async (buffer) => [...new Uint8Array(await crypto.subtle.digest('SHA-256', buffer))].map((b) => b.toString(16).padStart(2, '0')).join('');
  const shape = async (value) => {
    if (value && typeof value === 'object' && '$blob' in value) return new Blob([bytes(value.$blob)], { type: value.type });
    if (value && typeof value === 'object' && '$large' in value) {
      const data = new Uint8Array(value.$large * 1048576);
      for (let at = 0; at < data.length; at += 65536) crypto.getRandomValues(data.subarray(at, at + 65536));
      window.__largeSha = await hexOf(data);
      return new Blob([data], { type: 'application/octet-stream' });
    }
    if (value && typeof value === 'object' && '$cases' in value) {
      const file = bytes(value.$cases);
      return {
        proto: JSON.parse('{"__proto__": {"x": 1}, "y": 2}'),
        own: { $bruhs: 'own', a: 1 },
        holes: [1, undefined, 3],
        invalid: new Date(NaN),
        when: new Date(1763631000000),
        file: new File([file], 'note.txt', { type: 'text/plain' }),
      };
    }
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      const out = {};
      for (const [key, inner] of Object.entries(value)) out[key] = await shape(inner);
      return out;
    }
    return value;
  };
  const shaped = {};
  for (const [store, list] of Object.entries(records)) {
    shaped[store] = [];
    for (const record of list) shaped[store].push(await shape(record));
  }
  const db = await new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('the app has not made its database')); };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(new Error(String(open.error)));
  });
  await new Promise((resolve, reject) => {
    const tx = db.transaction(Object.keys(shaped), 'readwrite');
    for (const [store, list] of Object.entries(shaped)) for (const record of list) tx.objectStore(store).put(record);
    tx.addEventListener('complete', resolve);
    tx.addEventListener('error', () => reject(new Error(String(tx.error))));
    tx.addEventListener('abort', () => reject(new Error(String(tx.error))));
  });
  db.close();
  return true;
})()`;
// One store's records, or one record, through a connection of its own.
const readRecords = (store, key = null) => `(async () => {
  const db = await new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('no database')); };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(new Error(String(open.error)));
  });
  const value = await new Promise((resolve, reject) => {
    const objects = db.transaction(${js(store)}, 'readonly').objectStore(${js(store)});
    const read = ${key === null ? 'objects.getAll()' : `objects.get(${js(key)})`};
    read.onsuccess = () => resolve(read.result);
    read.onerror = () => reject(new Error(String(read.error)));
  });
  db.close();
  return value;
})()`;
// What a metadata record holds, as plain JSON: Blobs by type, size and
// SHA-256, the edge cases by how they read back.
const describeRecord = (key) => `(async () => {
  const record = await ${readRecords('metadata', '__KEY__').replace('"__KEY__"', js(key))};
  if (!record) return null;
  const hexOf = async (blob) => [...new Uint8Array(await crypto.subtle.digest('SHA-256', await blob.arrayBuffer()))].map((b) => b.toString(16).padStart(2, '0')).join('');
  const blob = async (value) => value instanceof Blob ? { blob: true, file: value instanceof File, type: value.type, size: value.size, sha: await hexOf(value) } : { blob: false };
  const value = record.value ?? {};
  const out = { key: record.key };
  if (value.draft) out.draft = await blob(value.draft);
  if (value.large) out.large = await blob(value.large);
  if (value.cases) {
    const cases = value.cases;
    const proto = Object.getOwnPropertyDescriptor(cases.proto ?? {}, '__proto__');
    out.cases = {
      protoOwn: !!proto, protoX: proto?.value?.x ?? null, protoPrototype: Object.getPrototypeOf(cases.proto ?? {}) === Object.prototype, protoY: cases.proto?.y ?? null,
      ownTag: cases.own?.$bruhs ?? null, ownA: cases.own?.a ?? null, ownKeys: Object.keys(cases.own ?? {}).sort(),
      holes: Array.isArray(cases.holes) ? { length: cases.holes.length, first: cases.holes[0], middleUndefined: cases.holes[1] === undefined, middlePresent: 1 in cases.holes, last: cases.holes[2] } : null,
      invalidDate: cases.invalid instanceof Date && Number.isNaN(cases.invalid.getTime()),
      when: cases.when instanceof Date ? cases.when.getTime() : null,
      file: await blob(cases.file),
    };
  }
  return out;
})()`;
// Every store, localStorage and sessionStorage as text (Blobs by size).
const storageText = `(async () => {
  const db = await new Promise((resolve, reject) => {
    const open = indexedDB.open('bruhs');
    open.onupgradeneeded = () => { open.transaction.abort(); reject(new Error('no database')); };
    open.onsuccess = () => resolve(open.result);
    open.onerror = () => reject(new Error(String(open.error)));
  });
  const parts = [];
  for (const name of db.objectStoreNames) {
    const all = await new Promise((resolve, reject) => {
      const read = db.transaction(name, 'readonly').objectStore(name).getAll();
      read.onsuccess = () => resolve(read.result);
      read.onerror = () => reject(new Error(String(read.error)));
    });
    const texts = [];
    const walk = async (value) => {
      if (value instanceof Blob) { texts.push(await value.text()); return; }
      if (value && typeof value === 'object') { for (const key of Object.keys(value)) { texts.push(key); await walk(value[key]); } return; }
      texts.push(String(value));
    };
    await walk(all);
    parts.push(name + ':' + texts.join('\\n'));
  }
  db.close();
  for (const store of [localStorage, sessionStorage]) for (let i = 0; i < store.length; i += 1) parts.push(store.key(i) + '=' + store.getItem(store.key(i)));
  return parts.join('\\n');
})()`;

let exitCode = 1;
try {
  // ── Servers and seed A ─────────────────────────────────────────────────────
  await startServer(A);
  await startServer(B);
  const command = (server, body) => api(server, 'POST', '/api/reactive/commands', body);
  const snapshot = (server) => api(server, 'GET', '/api/reactive');
  const overview = (server) => api(server, 'GET', '/api/backup');
  const bruhId = `backup-pix-${tag}`;
  const roomId = `room-${tag}`;
  const mailbox = `mbx-${tag}`;
  const audioId = `meeting-${tag}`;
  await command(A, {
    action: 'create_bruh',
    agent: { id: bruhId, name: 'Pix', instructions: 'Pix keeps notes for the owner.', model: 'claude-sonnet-4-6', project_path: '', enabled: true },
    look: { look: 'left', face: 'blueberry', mouth: 'bruh' },
    access: { level: 'ask', send_as_me: false, can_deliver: false },
  });
  await command(A, { action: 'create_room', room: { id: roomId, name: `Notes ${tag}`, members: [bruhId], lead: null } });
  const workA = await until("Pix's work folder", () => findDirs(A.core, (full, entry) => entry.isDirectory() && full.endsWith(path.join('bruhs', bruhId, 'work')))[0]);
  fs.writeFileSync(path.join(workA, `notes-${tag}.txt`), workText);
  const projectName = `Project ${tag}`;
  const globalEntry = await api(A, 'POST', '/api/vault', { name: `Global ${tag}`, scope: { type: 'global' }, secret: { kind: 'value', value: keychain.global } });
  await api(A, 'POST', '/api/vault', { name: projectName, scope: { type: 'project', path: projectA }, secret: { kind: 'value', value: keychain.project } });
  await api(A, 'POST', '/api/apps/accounts', { app: 'github', scope: { type: 'global' }, params: {}, token: keychain.app });
  const mailDb = (server) => path.join(server.core, 'server/mail.sqlite3');
  const insert = spawnSync('/usr/bin/sqlite3', [mailDb(A), `INSERT INTO mailboxes(id, provider, connection_id, revision) VALUES ('${mailbox}', 'gmail', 'conn-${tag}', 0);`], { encoding: 'utf8' });
  if (insert.status !== 0) throw new Error(`the mail row: ${insert.stderr}`);
  // The fixture mailbox has no connection behind it (a real one signs in
  // through OAuth), so the Apps listing can't read mail accounts on A
  // either; B must answer the same after the restore.
  const appsOnA = await request(A, 'GET', '/api/apps');
  const audioBytes = Buffer.concat([Buffer.from('RIFF'), randomBytes(64 * 1024)]);
  fs.mkdirSync(path.join(A.core, 'audio'), { recursive: true });
  fs.writeFileSync(path.join(A.core, 'audio', `${audioId}.wav`), audioBytes);
  fs.writeFileSync(path.join(A.core, 'audio', `${audioId}.jsonl`), `${js({ text: `hello ${tag}` })}\n`);
  // A Project Value keeps the legacy account, the SHA-256 of its path, a
  // zero byte and its name; an app token's item holds JSON with the token.
  const projectSlot = (projectPath) => sha256(Buffer.concat([Buffer.from(projectPath), Buffer.from([0]), Buffer.from(projectName)]));
  const holdsValue = (server, account, value) => items(server)[account] === value;
  const holdsApp = (server) => Object.values(items(server)).some((held) => held.includes(keychain.app));
  const seeded = {
    global: holdsValue(A, `vault:global:${globalEntry.id}`, keychain.global),
    project: holdsValue(A, projectSlot(projectA), keychain.project),
    app: holdsApp(A),
  };
  if (!seeded.global || !seeded.project || !seeded.app) throw new Error(`A's test keychain file doesn't hold the seeded values as expected: ${js(seeded)}; slots ${js(Object.keys(items(A)))}`);
  const protocol = (await snapshot(A)).protocol;
  const bAtStart = await snapshot(B);
  const bVaultAtStart = (await api(B, 'GET', '/api/vault')).entries.length;

  // ── Chrome ─────────────────────────────────────────────────────────────────
  chrome = await launchChrome({ port: cdpPort, profile: path.join(runDir, 'chrome') });
  const contextA = await newContext(chrome.browser);
  const contextB = await newContext(chrome.browser);
  await chrome.browser.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, browserContextId: contextA.id, eventsEnabled: true });
  await chrome.browser.send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: downloads, browserContextId: contextB.id, eventsEnabled: true });
  const pages = [];
  const panics = [];
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
      if (message.method === 'Network.requestWillBeSent') page.requested?.(message.params.request.method, new URL(message.params.request.url).pathname);
      if (message.method === 'Network.responseReceived') {
        const url = new URL(message.params.response.url);
        // Binary routes carry ciphertext; every other /api answer is kept.
        if (url.pathname.startsWith('/api/') && !url.pathname.startsWith('/api/backup/files/') && url.pathname !== '/api/backup/restore/web-part') responses.set(message.params.requestId, url.pathname);
        page.answered?.(url.pathname, message.params.response.status);
      }
      if (message.method === 'Network.loadingFinished' && responses.has(message.params.requestId)) {
        const url = responses.get(message.params.requestId);
        responses.delete(message.params.requestId);
        page.send('Network.getResponseBody', { requestId: message.params.requestId })
          .then(({ body, base64Encoded }) => bodies.push({ url, body: base64Encoded ? Buffer.from(body, 'base64').toString('utf8') : body }))
          .catch(() => {});
      }
    });
    // Upload bodies stay out of the events: a 140 MB body in one CDP
    // message breaks the connection.
    await page.send('Network.enable', { maxTotalBufferSize: 64 << 20, maxResourceBufferSize: 16 << 20, maxPostDataSize: 65536 });
    return page;
  };
  const frames = `window.__frames = 0; { const request = window.requestAnimationFrame.bind(window); window.requestAnimationFrame = (callback) => { window.__frames += 1; return request(callback); }; }
    addEventListener('pageshow', (event) => { (window.__shows ||= []).push(event.persisted); });`;
  const open = async (server, context, options = {}) => watch(await newPage(chrome.browser, server.base, { context, intercept: false, initScripts: [frames], ...options }));
  const pageA = await open(A, contextA);
  const pageB = await open(B, contextB);
  const settled = `!!document.querySelector('[data-backup-make]') && !!document.querySelector('[data-backup-restore]')`;
  // A fresh document each time, so no earlier job's lines linger.
  let loads = 0;
  const openSettings = async (page) => {
    loads += 1;
    await page.goto(`${page.origin}/?visit=${loads}#/settings`);
    if (!(await page.waitFor(settled, 60000))) throw new Error(`Settings, Backup didn't load on ${page.origin}`);
    await page.evaluate(`document.getElementById('backup').scrollIntoView({ block: 'start' })`);
    await sleep(300);
  };
  const shoot = (page, name) => page.screenshot(path.join(shots, `${name}.png`));
  const typeInto = async (page, selector, text) => {
    await page.evaluate(`document.querySelector(${js(selector)}).focus()`);
    await page.type(text);
  };
  const clickOn = async (page, selector) => {
    const clicked = await page.click(`document.querySelector(${js(selector)})`);
    if (!clicked.clicked) throw new Error(`couldn't click ${selector}: ${clicked.reason}`);
  };
  const text = (page, selector) => page.evaluate(`document.querySelector(${js(selector)})?.innerText ?? null`);
  // Whether focus lands on `selector` (focus moves after the next frame).
  const focusIs = (page, selector) => page.waitFor(`!!document.activeElement?.matches(${js(selector)})`, 3000);
  const focusedNow = (page) => page.evaluate(`(() => { const el = document.activeElement; return el ? el.tagName + [...el.attributes].filter((a) => a.name.startsWith('data-backup')).map((a) => '[' + a.name + ']').join('') + ' ' + el.innerText?.trim().slice(0, 30) : null; })()`);
  const SHIFT = 8;
  // Holds the desktop's instance lock, as a running desktop app does, so a
  // page's Replace is refused (QUIT_DESKTOP_FIRST).
  const holdInstanceLock = async (server) => {
    fs.mkdirSync(server.desktop, { recursive: true, mode: 0o700 });
    const file = path.join(server.desktop, 'desktop.instance.lock');
    const holder = spawn('python3', ['-c', 'import fcntl, os, sys, time\nfd = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT, 0o600)\nfcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)\nprint("held", flush=True)\ntime.sleep(600)', file], { stdio: ['ignore', 'pipe', 'ignore'] });
    children.add(holder);
    await new Promise((resolve, reject) => {
      holder.stdout.once('data', resolve);
      holder.once('exit', () => reject(new Error('the lock holder exited')));
    });
    return holder;
  };

  // Seed the web data: A's two threads, a project and the metadata records;
  // B's own earlier data, which a browser restore keeps aside.
  const draftBytes = randomBytes(200 * 1024);
  const fileBytes = Buffer.from(`file ${tag}`);
  const draftKey = `bruhs.qa.draft.${tag}`;
  const casesKey = `bruhs.qa.cases.${tag}`;
  const now = Date.now();
  await pageA.goto(`${A.base}/#/`);
  if (!(await pageA.appReady())) throw new Error("A's app didn't load");
  await pageA.evaluate(putRecords({
    threads: [
      { id: `t1-${tag}`, title: `One ${threadText}`, project_name: `App ${tag}`, updated_at: now, project_path: projectA },
      { id: `t2-${tag}`, title: `Two ${tag}`, project_name: '', updated_at: now - 1000 },
    ],
    messages: [{ thread_id: `t1-${tag}`, messages: [{ role: 'user', content: threadText }] }],
    projects: [{ id: `p-${tag}`, name: `App ${tag}`, color: null, path: projectA }],
    metadata: [
      { key: draftKey, revision: 1, value: { text: `draft ${tag}`, draft: { $blob: draftBytes.toString('base64'), type: 'application/x-bruhs-draft' } } },
      { key: casesKey, revision: 1, value: { cases: { $cases: fileBytes.toString('base64') } } },
    ],
  }));
  const model = `canary-model-${tag}`;
  await pageA.evaluate(`localStorage.setItem('bruhs.model', ${js(js(model))})`);
  const casesOnA = await pageA.evaluate(describeRecord(casesKey));
  const draftOnA = await pageA.evaluate(describeRecord(draftKey));
  await pageB.goto(`${B.base}/#/`);
  if (!(await pageB.appReady())) throw new Error("B's app didn't load");
  await pageB.evaluate(putRecords({ threads: [{ id: `b-${tag}`, title: `B before ${tag}`, project_name: '', updated_at: now }] }));
  await pageB.evaluate(`localStorage.setItem('bruhs.qa.b', 'before')`);

  // 1. Settings, Backup on A.
  await check('Settings, Backup on A: after People, the option labels with sizes, WEB_INCLUDED, no horizontal scroll at 1280×800 and 390×844', async () => {
    await openSettings(pageA);
    const state = await pageA.evaluate(`(() => {
      const nav = [...document.querySelectorAll('nav[aria-label="Settings sections"] button')].map((b) => b.innerText.trim());
      const ids = [...document.querySelectorAll('section[id], [id="apps"], [id="backup"]')].map((n) => n.id).filter(Boolean);
      return {
        nav, people: nav.indexOf('People'), backup: nav.indexOf('Backup'),
        afterPeople: !!document.getElementById('people') && !!(document.getElementById('people').compareDocumentPosition(document.getElementById('backup')) & Node.DOCUMENT_POSITION_FOLLOWING),
        options: [...document.querySelectorAll('[data-backup-option] p:first-child')].map((p) => p.innerText.trim()),
        web: document.querySelector('[data-backup-web-included]')?.innerText.trim(),
        sideways: document.scrollingElement.scrollWidth - innerWidth,
      };
    })()`);
    const listed = await overview(A);
    await shoot(pageA, 'a-backup-1280x800');
    await pageA.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
    await pageA.evaluate(`document.getElementById('backup').scrollIntoView({ block: 'start' })`);
    await sleep(300);
    const narrow = await pageA.evaluate(`document.scrollingElement.scrollWidth - innerWidth`);
    await shoot(pageA, 'a-backup-390x844');
    await pageA.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
    const size = /\(\d+(\.\d+)? (B|KB|MB|GB)\)$/;
    const [keys, audio, browsers] = state.options;
    record('Settings, Backup on A: after People, the option labels with sizes, WEB_INCLUDED, no horizontal scroll at 1280×800 and 390×844',
      state.backup === state.people + 1 && state.afterPeople && keys === `Saved logins, keys and app tokens (${listed.keychain_items})` && listed.keychain_items === 3
        && audio?.startsWith('Meeting audio (') && size.test(audio) && browsers?.startsWith('Bruh browser sign-ins (') && size.test(browsers)
        && state.web === COPY.webIncluded && state.sideways <= 0 && narrow <= 0,
      `nav ${js(state.nav)}; options ${js(state.options)}; keychain items ${listed.keychain_items}; audio bytes ${listed.sizes.audio_bytes}; web ${js(state.web)}; sideways ${state.sideways} at 1280, ${narrow} at 390`);
  });

  // 2. Make a backup by keyboard.
  let backupFile = null;
  let downloadId = null;
  await check('Make a backup by keyboard: a mismatched second password shows PASSWORDS_DIFFER and sends nothing, a good one makes it, progress lines appear, one .bruhsbak downloads', async () => {
    await openSettings(pageA);
    const sent = [];
    pageA.requested = (method, pathname) => sent.push(`${method} ${pathname}`);
    await typeInto(pageA, '[data-backup-make] input[type=password]', password);
    await pageA.press('Tab');
    const second = await pageA.evaluate(`document.activeElement === document.querySelectorAll('[data-backup-make] input[type=password]')[1]`);
    await pageA.type(`${password}x`);
    await sleep(200);
    const differ = await text(pageA, '[data-backup-second-error]');
    await pageA.press('Enter');
    await sleep(800);
    const sentOnMismatch = sent.filter((route) => route === 'POST /api/backup' || route === 'POST /api/backup/web-parts').length;
    await shoot(pageA, 'a-backup-mismatch-1280x800');
    await pageA.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: process.platform === 'darwin' ? 4 : 2, commands: ['selectAll'] });
    await pageA.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: process.platform === 'darwin' ? 4 : 2 });
    await pageA.type(password);
    await sleep(200);
    const cleared = await text(pageA, '[data-backup-second-error]');
    await pageA.evaluate(`(() => {
      window.__lines = [];
      const note = () => { const line = document.querySelector('[data-backup-make-status]')?.innerText.trim(); if (line && window.__lines.at(-1) !== line) window.__lines.push(line); };
      new MutationObserver(note).observe(document.body, { subtree: true, childList: true, characterData: true });
    })()`);
    const before = new Set(fs.readdirSync(downloads));
    const peak = await peakRenderer(async () => {
      await pageA.press('Enter');
      if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-made]') || !!document.querySelector('[data-backup-make-error]')?.innerText.trim()`, 180000))) throw new Error('the backup never finished');
    });
    measured.smallDump = peak;
    const made = await text(pageA, '[data-backup-make-status]');
    const error = await text(pageA, '[data-backup-make-error]');
    const lines = await pageA.evaluate('window.__lines');
    const href = await pageA.evaluate(`document.querySelector('[data-backup-download]')?.getAttribute('href') ?? null`);
    downloadId = href?.split('/').pop() ?? null;
    const fields = await pageA.evaluate(`[...document.querySelectorAll('[data-backup-make] input[type=password]')].map((i) => i.value.length)`);
    const file = await until('the download', () => {
      const fresh = fs.readdirSync(downloads).filter((name) => !before.has(name));
      return fresh.length === 1 && fresh[0].endsWith('.bruhsbak') ? fresh[0] : null;
    }, 60000).catch(() => null);
    const all = fs.readdirSync(downloads).filter((name) => !before.has(name));
    if (file) backupFile = path.join(downloads, file);
    await shoot(pageA, 'a-backup-made-1280x800');
    pageA.requested = null;
    record('Make a backup by keyboard: a mismatched second password shows PASSWORDS_DIFFER and sends nothing, a good one makes it, progress lines appear, one .bruhsbak downloads',
      second && differ === COPY.passwordsDiffer && sentOnMismatch === 0 && !cleared && !error && /^Backup ready: bruhs-backup-\d{4}-\d{2}-\d{2}-\d{4}\.bruhsbak, .+\.$/.test(made ?? '')
        && lines.includes(COPY.locking) && lines.some((line) => /^(Reading your data|Reading your keychain|Writing the backup)/.test(line)) && fields.every((length) => length === 0)
        && !!file && all.length === 1 && /^[0-9a-f]{32}$/.test(downloadId ?? ''),
      `Tab reached the second field ${second}; mismatch ${js(differ)}; sent on mismatch ${sentOnMismatch}; error ${js(error)}; result ${js(made)}; lines ${js(lines)}; fields ${js(fields)}; downloads ${js(all)}; renderer ${js(peak)}`);
  });
  if (!backupFile) throw new Error('No backup file to go on with.');
  const backupBytes = fs.readFileSync(backupFile);

  // 3. Nothing in clear.
  await check('The file starts with BRUHSBAK; no keychain canary, password, thread or work text, or path appears in it in clear', async () => {
    const clear = [...secretSet, ['threadText', threadText], ['workText', workText], ['home', A.home], ['homeName', 'home-a'], ['project', projectA], ['tag', tag]]
      .filter(([, value]) => backupBytes.includes(Buffer.from(value))).map(([name]) => name);
    record('The file starts with BRUHSBAK; no keychain canary, password, thread or work text, or path appears in it in clear',
      backupBytes.subarray(0, 8).toString('latin1') === 'BRUHSBAK' && clear.length === 0,
      `${backupBytes.length} bytes; magic ${js(backupBytes.subarray(0, 8).toString('latin1'))}; in clear ${js(clear)}`);
  });

  // 4. Owner only.
  await check('A page with another loopback Origin gets 403 OWNER_ONLY_BACKUP on POST /api/backup, POST /api/backup/restore/apply and GET /api/backup/files/<id>', async () => {
    const foreign = `http://127.0.0.1:${A.port + 2}`;
    const answers = [
      await request(A, 'POST', '/api/backup', { password, options: { keychain: true, audio: true, browser_profiles: false }, app: 'web' }, { origin: foreign }),
      await request(A, 'POST', '/api/backup/restore/apply', { source: { type: 'staged', id: '0'.repeat(32) }, replace: true }, { origin: foreign }),
      await request(A, 'GET', `/api/backup/files/${downloadId}`, null, { origin: foreign }),
    ];
    record('A page with another loopback Origin gets 403 OWNER_ONLY_BACKUP on POST /api/backup, POST /api/backup/restore/apply and GET /api/backup/files/<id>',
      answers.every((answer) => answer.status === 403 && answer.body?.error === COPY.ownerOnly),
      answers.map((answer) => `${answer.status} ${answer.text.slice(0, 120)}`).join(' | '));
  });

  // 5. Leaks on disk.
  await check("No file under home-a (but the D26 keys folder, outside both roots), /tmp or A's server log holds a keychain canary or the password", async () => {
    const keysOutside = ![A.core, A.desktop].some((rootDir) => path.relative(rootDir, A.keys).split(path.sep)[0] !== '..');
    const home = fileLeaks(filesUnder(A.home, { skip: (full) => full === A.keys }));
    const temp = fileLeaks(filesUnder('/private/tmp', { since: runStart, maxBytes: 64 << 20 }));
    const log = fileLeaks([A.log]);
    const keysHold = fileLeaks([path.join(A.keys, 'items.json')], keychainSet).length;
    record("No file under home-a (but the D26 keys folder, outside both roots), /tmp or A's server log holds a keychain canary or the password",
      keysOutside && home.length === 0 && temp.length === 0 && log.length === 0 && keysHold === 3,
      `keys folder outside both roots ${keysOutside}; home ${js(home)}; /tmp ${js(temp)}; log ${js(log)}; the keys file holds ${keysHold} of 3 (by design)`);
  });

  // 6. Open on B and replace.
  const restoreForm = '[data-backup-restore]';
  const chooseFile = async (page, file) => {
    const { root: doc } = await page.send('DOM.getDocument', { depth: 0 });
    const { nodeId } = await page.send('DOM.querySelector', { nodeId: doc.nodeId, selector: 'input[type=file][accept=".bruhsbak"]' });
    await page.send('DOM.setFileInputFiles', { nodeId, files: [file] });
    if (!(await page.waitFor(`document.querySelector('[data-backup-chosen]')?.innerText === ${js(path.basename(file))}`, 5000))) throw new Error('the file was not chosen');
  };
  const openBackup = async (page, file, secret) => {
    await chooseFile(page, file);
    await typeInto(page, `${restoreForm} input[type=password]`, secret);
    await clickOn(page, '[data-backup-open]');
  };
  // Replace (or confirm a Put back), then play "start Bruhs again": the
  // server exits, the script starts it, and the page says it's back.
  const replaceAndRestart = async (server, page, press, label) => {
    let answeredAt = null;
    page.answered = (pathname, status) => { if (pathname === '/api/backup/restore/apply' && status === 200) answeredAt = Date.now(); };
    const pressedAt = Date.now();
    await press();
    const stopping = await page.waitFor(`!!document.querySelector('[data-backup-stopping]')`, 15000);
    const stoppingText = await text(page, '[data-backup-stopping]');
    await shoot(page, `${label}-stopping-1280x800`);
    const exit = await Promise.race([server.exited, sleep(90000).then(() => null)]);
    if (!exit) throw new Error(`${label}: bruhs-server ${server.name} didn't exit`);
    const startMs = await startServer(server);
    const back = await page.waitFor(`!!document.querySelector('[data-backup-restored]')`, 90000);
    page.answered = null;
    // The detached applier should be gone soon after the data is in place.
    const healthyAt = Date.now();
    const applierGone = await until('the applier exits', () => (appliers().length === 0 ? true : null), 60000, 250).catch(() => false);
    const timing = { exitAfterAnswerMs: answeredAt ? exit.at - answeredAt : null, exitAfterPressMs: exit.at - pressedAt, exitCode: exit.code, exitSignal: exit.signal, restartToHealthyMs: startMs, applierGoneAfterHealthyMs: applierGone ? Date.now() - healthyAt : null };
    measured[`${label}Exit`] = timing;
    return { stopping, stoppingText, back: back && applierGone === true, timing, restoredText: await text(page, '[data-backup-restored]') };
  };
  let replaced = null;
  await check('On B: a wrong password says WRONG_PASSWORD; the right one previews the path rewrite, counts and browser data, focused on Cancel; Cancel by keyboard discards it and focuses Choose backup file; a refused Replace keeps focus; Replace my data: STOPPING, B exits, restarts and says RESTORED', async () => {
    await openSettings(pageB);
    await openBackup(pageB, backupFile, wrongPassword);
    await pageB.waitFor(`!!document.querySelector('[data-backup-restore-error]')?.innerText.trim()`, 60000);
    const wrong = (await text(pageB, '[data-backup-restore-error]'))?.trim();
    await openBackup(pageB, backupFile, password);
    if (!(await pageB.waitFor(`!!document.querySelector('[data-backup-preview]')`, 120000))) throw new Error(`no preview: ${await text(pageB, '[data-backup-restore-error]')}`);
    const lines = await pageB.evaluate(`[...document.querySelectorAll('[data-backup-preview-line]')].map((li) => li.innerText.trim())`);
    const fieldEmpty = await pageB.evaluate(`[...document.querySelectorAll('${restoreForm} input[type=password]')].every((i) => i.value === '')`);
    await pageB.evaluate(`document.querySelector('[data-backup-preview]').scrollIntoView({ block: 'center' })`);
    await shoot(pageB, 'b-preview-1280x800');
    const stagedBefore = (await overview(B)).staged?.preview?.id ?? null;
    // D-I2: a new preview focuses its Cancel; Cancel by keyboard hands
    // focus to Choose backup file.
    const focus = { preview: await focusIs(pageB, '[data-backup-preview] [data-backup-cancel]'), previewAt: await focusedNow(pageB) };
    await pageB.press('Enter');
    await pageB.waitFor(`!document.querySelector('[data-backup-preview]')`, 15000);
    focus.choose = await focusIs(pageB, '[data-backup-choose]');
    focus.chooseAt = await focusedNow(pageB);
    const afterCancel = await until('the staged set goes', async () => ((await overview(B)).staged === null ? true : null), 15000).catch(() => false);
    const stagingLeft = (() => {
      try {
        return fs.readdirSync(path.join(B.core, 'backups/staging')).length;
      } catch {
        return 0;
      }
    })();
    await openBackup(pageB, backupFile, password);
    if (!(await pageB.waitFor(`!!document.querySelector('[data-backup-preview]')`, 120000))) throw new Error('no preview the second time');
    focus.secondPreview = await focusIs(pageB, '[data-backup-preview] [data-backup-cancel]');
    // A refused Replace, by keyboard, keeps focus on Replace and the preview.
    const holder = await holdInstanceLock(B);
    await pageB.press('Tab', SHIFT);
    focus.onReplace = await focusIs(pageB, '[data-backup-replace]');
    await pageB.press('Enter');
    await pageB.waitFor(`!!document.querySelector('[data-backup-restore-error]')?.innerText.trim()`, 15000);
    const refused = (await text(pageB, '[data-backup-restore-error]'))?.trim();
    focus.stayed = await focusIs(pageB, '[data-backup-replace]');
    focus.stayedAt = await focusedNow(pageB);
    focus.previewKept = await pageB.evaluate(`!!document.querySelector('[data-backup-preview]')`);
    await stopChild(holder);
    measured.restoreFocus = { ...focus, refused };

    // Other tabs on B while the restore runs: one with an unsent draft, one
    // that went back and forward (see check 8).
    const draftTab = await open(B, contextB);
    const dialogs = [];
    draftTab.handlers.add((message) => {
      if (message.method === 'Page.javascriptDialogOpening') {
        dialogs.push(message.params.type);
        draftTab.send('Page.handleJavaScriptDialog', { accept: false }).catch(() => {});
      }
    });
    await draftTab.goto(`${B.base}/#/`);
    await draftTab.appReady();
    await draftTab.waitFor(`!!document.querySelector('textarea')`, 30000);
    await draftTab.click(`document.querySelector('textarea')`);
    await draftTab.type(`an unsent draft ${tag}`);
    await sleep(400);
    await draftTab.evaluate(`setTimeout(() => location.reload(), 0)`);
    await sleep(1500);
    const guardArmed = dialogs.includes('beforeunload');
    dialogs.length = 0;
    await draftTab.evaluate(`window.__stale = 1`);
    const cachedTab = await open(B, contextB);
    const notUsed = [];
    cachedTab.handlers.add((message) => { if (message.method === 'Page.backForwardCacheNotUsed') notUsed.push(...(message.params.notRestoredExplanations ?? []).map((entry) => entry.reason)); });
    await cachedTab.goto(`${B.base}/#/`);
    await cachedTab.appReady();
    await sleep(800);
    await cachedTab.goto(`${B.base}/__nothing-${tag}`);
    await sleep(800);
    replaced = { draftTab, dialogs, guardArmed, cachedTab, notUsed };

    const outcome = await replaceAndRestart(B, pageB, () => clickOn(pageB, '[data-backup-replace]'), 'b-restore');
    replaced.outcome = outcome;
    await shoot(pageB, 'b-restored-1280x800');
    const rewrite = `Paths under ${A.home} become ${B.home}.`;
    record('On B: a wrong password says WRONG_PASSWORD; the right one previews the path rewrite, counts and browser data, focused on Cancel; Cancel by keyboard discards it and focuses Choose backup file; a refused Replace keeps focus; Replace my data: STOPPING, B exits, restarts and says RESTORED',
      wrong === COPY.wrongPassword && lines.includes(rewrite) && lines.some((line) => /^1 bruh, 3 vault entries, 3 keychain items, \d+ files, .+\.$/.test(line)) && lines.includes(COPY.webIncludedPreview)
        && fieldEmpty && !!stagedBefore && afterCancel === true && stagingLeft === 0
        && focus.preview && focus.choose && focus.secondPreview && focus.onReplace && refused === COPY.quitDesktop && focus.stayed && focus.previewKept
        && outcome.stopping && outcome.stoppingText?.includes(COPY.stopping) && outcome.back && outcome.restoredText?.includes(COPY.restored),
      `wrong ${js(wrong)}; preview ${js(lines)}; field emptied ${fieldEmpty}; focus ${js(measured.restoreFocus)}; staged ${!!stagedBefore}, after Cancel gone ${afterCancel}, staging entries ${stagingLeft}; stopping ${js(outcome.stoppingText)}; restored ${js(outcome.restoredText)}; timing ${js(outcome.timing)}`);
  });

  // 7. B's server data after the restart.
  await check("On B after restart: the bruh, paused, with its work folder file under home-b-other; the room and the mail row; both vault values check out and hold their canaries; the Project Value is under the new path's hash; one kept set", async () => {
    const state = await snapshot(B);
    const bruh = state.bruhs.find((entry) => entry.agent_id === bruhId);
    const room = state.rooms.find((entry) => (entry.room?.id ?? entry.id) === roomId);
    const work = findDirs(B.core, (full, entry) => entry.isDirectory() && full.endsWith(path.join('bruhs', bruhId, 'work')))[0] ?? null;
    const workFile = work ? fs.readFileSync(path.join(work, `notes-${tag}.txt`), 'utf8') : null;
    const mail = sqlite(mailDb(B), `SELECT count(*) FROM mailboxes WHERE id = '${mailbox}';`).stdout.trim();
    const vault = await api(B, 'GET', '/api/vault');
    const projectOnB = vault.entries.find((candidate) => candidate.name === projectName);
    const checks = await Promise.all([globalEntry.id, projectOnB?.id].filter(Boolean).map((id) => api(B, 'POST', `/api/vault/${id}/check`)));
    const projectScope = projectOnB?.scope;
    const values = {
      global: holdsValue(B, `vault:global:${globalEntry.id}`, keychain.global),
      project: holdsValue(B, projectSlot(projectB), keychain.project),
      projectOldKey: projectSlot(projectA) in items(B),
      app: holdsApp(B),
    };
    const apps = await request(B, 'GET', '/api/apps');
    const appEntry = vault.entries.find((candidate) => candidate.kind?.kind === 'app_token' && candidate.kind?.app === 'github');
    const audio = (() => {
      try {
        return sha256(fs.readFileSync(path.join(B.core, 'audio', `${audioId}.wav`))) === sha256(audioBytes);
      } catch {
        return false;
      }
    })();
    const kept = await overview(B);
    record("On B after restart: the bruh, paused, with its work folder file under home-b-other; the room and the mail row; both vault values check out and hold their canaries; the Project Value is under the new path's hash; one kept set",
      bruh?.status === 'paused' && !!work && work.startsWith(B.home) && workFile === workText && !!room && mail === '1'
        && checks.length === 2 && checks.every((answer) => answer.ok === true) && projectScope?.type === 'project' && projectScope?.path === projectB
        && values.global && values.project && !values.projectOldKey && values.app && !!appEntry && apps.status === appsOnA.status && apps.text === appsOnA.text
        && audio && kept.pre_restores.length === 1 && (await snapshot(B)).protocol === protocol,
      `bruh ${js(bruh?.status)}; work folder under B ${work?.startsWith(B.home)}, file ${workFile === workText}; room ${!!room}; mail rows ${mail}; checks ${js(checks.map((answer) => answer.ok))}; project scope ${js(projectScope)}; values ${js(values)}; app token entry ${!!appEntry}; Apps listing on B ${apps.status} ${apps.text.slice(0, 80)}, on A ${appsOnA.status} ${appsOnA.text.slice(0, 80)}; audio ${audio}; kept sets ${kept.pre_restores.length}`);
  });

  // 8. The browser part.
  await check("The browser-data card restores IndexedDB and localStorage: both threads, the project's path rewritten, the Blob byte for byte, the edge cases, bruhs.model; B's previous browser data goes to the kept set; the draft tab reloads without asking; the back and forward tab shows no stale data", async () => {
    if (!(await pageB.waitFor(`!!document.querySelector('[data-backup-browser]')`, 20000))) throw new Error('no browser-data card');
    const card = await pageB.evaluate(`({ field: !!document.querySelector('[data-backup-browser] input[type=password]'), text: document.querySelector('[data-backup-browser]').innerText })`);
    await pageB.evaluate(`document.querySelector('[data-backup-browser]').scrollIntoView({ block: 'center' })`);
    await shoot(pageB, 'b-browser-card-1280x800');
    // The restore up to its done line, then the tab's own reload (a debug
    // bundle's wasm is large, so a reload is measured on its own).
    await pageB.evaluate(`window.__beforeRestore = 1`);
    const peak = await peakRenderer(async () => {
      await clickOn(pageB, '[data-backup-restore-browser]');
      await pageB.waitFor(`!!document.querySelector('[data-backup-browser-done]') || window.__beforeRestore !== 1`, 180000);
    });
    const reloadPeak = await peakRenderer(async () => {
      await pageB.waitFor(`window.__beforeRestore === undefined && document.readyState === 'complete'`, 60000);
      await pageB.appReady();
      await sleep(1500);
    });
    measured.smallRestore = peak;
    measured.smallRestoreReload = reloadPeak;
    const threads = await pageB.evaluate(readRecords('threads'));
    const projects = await pageB.evaluate(readRecords('projects'));
    const draft = await pageB.evaluate(describeRecord(draftKey));
    const cases = await pageB.evaluate(describeRecord(casesKey));
    const storedModel = await pageB.evaluate(`localStorage.getItem('bruhs.model')`);
    const bMarker = await pageB.evaluate(`localStorage.getItem('bruhs.qa.b')`);
    const one = threads.find((thread) => thread.id === `t1-${tag}`);
    const project = projects.find((entry) => entry.id === `p-${tag}`);
    const kept = await overview(B);
    // Cases compared as they read on A, but for the File, which comes back
    // as a Blob (documented narrowing).
    const sameCases = js({ ...cases?.cases, file: { ...cases?.cases?.file, file: undefined } }) === js({ ...casesOnA?.cases, file: { ...casesOnA?.cases?.file, file: undefined } });
    const { draftTab, dialogs, guardArmed, cachedTab, notUsed } = replaced;
    await sleep(1500);
    const draftReloaded = await draftTab.evaluate(`window.__stale === undefined`).catch(() => false);
    const history = await cachedTab.send('Page.getNavigationHistory');
    await cachedTab.send('Page.navigateToHistoryEntry', { entryId: history.entries[history.currentIndex - 1].id });
    await sleep(2500);
    await cachedTab.appReady();
    const back = await cachedTab.evaluate(`({ shows: window.__shows ?? [], href: location.href, text: document.body.innerText })`);
    const fromCache = back.shows.at(-1) === true;
    const shows = { restored: back.text.includes(`Two ${tag}`), stale: back.text.includes(`B before ${tag}`) };
    measured.bfcache = { fromCache, reasons: [...new Set(notUsed)], shows };
    record("The browser-data card restores IndexedDB and localStorage: both threads, the project's path rewritten, the Blob byte for byte, the edge cases, bruhs.model; B's previous browser data goes to the kept set; the draft tab reloads without asking; the back and forward tab shows no stale data",
      !card.field && threads.some((thread) => thread.id === `t2-${tag}`) && !!one && !threads.some((thread) => thread.id === `b-${tag}`) && one?.project_path === projectB && project?.path === projectB
        && draft?.draft?.sha === sha256(draftBytes) && draft?.draft?.type === 'application/x-bruhs-draft' && js(draft) === js(draftOnA)
        && sameCases && cases?.cases?.file?.blob && cases?.cases?.file?.sha === sha256(fileBytes) && cases?.cases?.protoOwn && cases?.cases?.holes?.middleUndefined && cases?.cases?.invalidDate
        && storedModel === js(model) && bMarker === null && kept.pre_restores.length === 1 && kept.pre_restores[0].has_web_part === true
        && guardArmed && draftReloaded && dialogs.length === 0 && !(fromCache && shows.stale) && shows.restored,
      `card has a field ${card.field}; threads ${js(threads.map((thread) => thread.id.split('-')[0]))}; thread path rewritten ${one?.project_path === projectB}; project path rewritten ${project?.path === projectB}; draft ${js(draft?.draft && { ...draft.draft, sha: draft.draft.sha === sha256(draftBytes) })}; cases on A ${js(casesOnA?.cases)}; on B ${js(cases?.cases)}; model ${storedModel === js(model)}; B's own marker gone ${bMarker === null}; kept web part ${kept.pre_restores[0]?.has_web_part}; draft guard armed ${guardArmed}, tab reloaded ${draftReloaded}, dialogs ${js(dialogs)}; back and forward tab from cache ${fromCache}, reasons ${js(measured.bfcache.reasons)}, shows ${js(shows)}; renderer ${js(peak)}, its reload ${js(reloadPeak)}`);
  });

  // 9. No values in B's browser or any answer.
  await check("B's IndexedDB, localStorage and sessionStorage hold no keychain canary or password; no response body during the run held a keychain canary", async () => {
    const storage = found(await pageB.evaluate(storageText));
    const answers = bodies.flatMap((entry) => found(entry.body, keychainSet).map((name) => `${entry.url}:${name}`));
    record("B's IndexedDB, localStorage and sessionStorage hold no keychain canary or password; no response body during the run held a keychain canary",
      storage.length === 0 && answers.length === 0 && bodies.length > 20,
      `storage ${js(storage)}; responses ${js(answers)} of ${bodies.length}`);
  });

  // A plain reload of the app, for comparison with the peaks above.
  measured.plainReload = await peakRenderer(async () => {
    await pageB.reload();
    await pageB.appReady();
    await sleep(1500);
  });

  // 10. Put back, then delete.
  await check("Put back on B restores B's earlier (empty) data and keeps the restored set aside; deleting it removes its folder and seal key; each confirm takes focus on Keep and Escape hands it back", async () => {
    const before = await overview(B);
    const stamp = before.pre_restores[0]?.stamp;
    await openSettings(pageB);
    // Each confirm takes focus on its own Keep; Escape hands it back.
    const confirmFocus = async (button) => {
      await pageB.evaluate(`document.querySelector('${button}').focus()`);
      await pageB.press('Enter');
      await pageB.waitFor(`!!document.querySelector('[data-backup-kept-confirm]')`, 5000);
      const keep = await focusIs(pageB, '[data-backup-kept-confirm] [data-backup-decline]');
      const keepAt = await focusedNow(pageB);
      await pageB.press('Escape');
      const back = await focusIs(pageB, button) && (await pageB.evaluate(`!document.querySelector('[data-backup-kept-confirm]')`));
      await pageB.press('Enter');
      await pageB.waitFor(`!!document.querySelector('[data-backup-kept-confirm]')`, 5000);
      return { keep, keepAt, back };
    };
    const putBackFocus = await confirmFocus('[data-backup-put-back]');
    await shoot(pageB, 'b-put-back-confirm-1280x800');
    const outcome = await replaceAndRestart(B, pageB, () => clickOn(pageB, '[data-backup-confirm]'), 'b-put-back');
    const state = await snapshot(B);
    const vault = await api(B, 'GET', '/api/vault');
    const mail = sqlite(mailDb(B), `SELECT count(*) FROM mailboxes WHERE id = '${mailbox}';`).stdout.trim();
    const after = await overview(B);
    const restoredSet = after.pre_restores.find((set) => set.stamp !== stamp) ?? null;
    const sealSlots = (setStamp) => Object.keys(items(B)).filter((slot) => slot.includes('bruhs-ai-backup') && slot.includes(setStamp));
    const sealBefore = restoredSet ? sealSlots(restoredSet.stamp).length : 0;
    const folders = (setStamp) => [B.core, B.desktop].map((rootDir) => path.join(rootDir, 'backups', `pre-restore-${setStamp}`)).filter((folder) => fs.existsSync(folder));
    const foldersBefore = restoredSet ? folders(restoredSet.stamp).length : 0;
    await openSettings(pageB);
    const deleteFocus = await confirmFocus('[data-backup-delete]');
    await clickOn(pageB, '[data-backup-confirm]');
    const gone = await until('the set goes', async () => ((await overview(B)).pre_restores.length === 0 ? true : null), 20000).catch(() => false);
    await shoot(pageB, 'b-after-delete-1280x800');
    record("Put back on B restores B's earlier (empty) data and keeps the restored set aside; deleting it removes its folder and seal key; each confirm takes focus on Keep and Escape hands it back",
      !!stamp && outcome.back && js(state.bruhs.map((bruh) => bruh.agent_id)) === js(bAtStart.bruhs.map((bruh) => bruh.agent_id)) && state.rooms.length === bAtStart.rooms.length && vault.entries.length === bVaultAtStart && mail === '0'
        && after.pre_restores.length === 1 && !!restoredSet && sealBefore >= 1 && foldersBefore >= 1
        && gone === true && folders(restoredSet.stamp).length === 0 && sealSlots(restoredSet.stamp).length === 0
        && putBackFocus.keep && putBackFocus.back && deleteFocus.keep && deleteFocus.back,
      `confirm focus: Put back ${js(putBackFocus)}, Delete ${js(deleteFocus)}; put back ${js(outcome.restoredText)}; bruhs ${state.bruhs.length}, rooms ${state.rooms.length}, vault ${vault.entries.length}, mail rows ${mail}; kept after ${after.pre_restores.length}; seal slots before delete ${sealBefore}, folders ${foldersBefore}; deleted ${gone}, folders left ${restoredSet ? folders(restoredSet.stamp).length : '?'}, seal slots left ${restoredSet ? sealSlots(restoredSet.stamp).length : '?'}; backup slots ${js(Object.keys(items(B)).filter((slot) => slot.includes('bruhs-ai-backup')))}; timing ${js(outcome.timing)}`);
  });

  // 11. A crafted archive.
  await check("A hand-crafted archive (same format and password, Argon2id at FLOOR) holding core/../../escape is refused with UNSAFE_ENTRY, and escape exists nowhere under either home", async () => {
    const openCrafted = async (bytes) => {
      const upload = await request(B, 'POST', '/api/backup/restore/uploads', null, { raw: bytes });
      if (upload.status !== 200) return `upload ${upload.status} ${upload.text}`;
      const job = await api(B, 'POST', '/api/backup/restore', { source: { type: 'upload', id: upload.body.id }, password });
      const done = await until('the open job', async () => {
        const state = (await api(B, 'GET', `/api/backup/jobs/${job.id}`)).state;
        return state.state === 'running' ? null : state;
      }, 60000);
      if (done.state === 'opened') await request(B, 'DELETE', `/api/backup/restore/${done.preview.id}`);
      return done.state === 'failed' ? done.message : done.state;
    };
    // A control with a safe name and no manifest proves the format is right:
    // it opens, then fails for want of a manifest, not on its password.
    const control = await openCrafted(craftArchive(password, [['core/ok.txt', 'fine']]));
    const escape = await openCrafted(craftArchive(password, [['core/../../escape', 'escape']]));
    const strays = [A.home, B.home, runDir].flatMap((dir) => findDirs(dir, (full, entry) => entry.name === 'escape'));
    record("A hand-crafted archive (same format and password, Argon2id at FLOOR) holding core/../../escape is refused with UNSAFE_ENTRY, and escape exists nowhere under either home",
      control !== COPY.wrongPassword && control !== COPY.unsafeEntry && escape === COPY.unsafeEntry && strays.length === 0,
      `control ${js(control)}; crafted ${js(escape)}; strays ${js(strays)}`);
  });

  // 12. Phone size, reduced motion, panics.
  await check("390×844 touch: Backup's controls (the shared Switch apart) are at least 44px and there is no horizontal page scroll; reduced motion leaves no animation frames", async () => {
    const phone = await open(B, contextB, { width: 390, height: 844, touch: true });
    await openSettings(phone);
    if (await phone.evaluate(`!!document.querySelector('div.fixed.inset-0.z-30')`)) {
      for (const type of ['mousePressed', 'mouseReleased']) await phone.send('Input.dispatchMouseEvent', { type, x: 385, y: 600, button: 'left', clickCount: 1 });
      await phone.waitFor(`!document.querySelector('div.fixed.inset-0.z-30')`, 3000);
      await phone.evaluate(`document.getElementById('backup').scrollIntoView({ block: 'start' })`);
    }
    await sleep(400);
    const layout = await phone.evaluate(`(() => {
      const section = document.getElementById('backup');
      const shown = (node) => node.getBoundingClientRect().height > 0 && !node.closest('.sr-only') && node.getAttribute('aria-hidden') !== 'true';
      const label = (node) => node.getAttribute('aria-label') || node.labels?.[0]?.textContent.trim() || node.textContent.trim().slice(0, 30);
      const size = (node) => Math.round(node.closest('label.bruhs-control')?.getBoundingClientRect().height ?? node.getBoundingClientRect().height);
      const controls = [...section.querySelectorAll('button, input, a')].filter(shown).map((node) => ({ label: label(node), role: node.getAttribute('role'), height: size(node), left: Math.round(node.getBoundingClientRect().left), right: Math.round(node.getBoundingClientRect().right) }));
      return { controls, width: innerWidth, sideways: document.scrollingElement.scrollWidth - innerWidth, coarse: matchMedia('(pointer: coarse)').matches };
    })()`);
    await shoot(phone, 'b-backup-390x844-touch');
    // The shared Switch (ui/switch.rs, every Settings section) is 24px tall
    // on touch; it is recorded apart, not counted against Backup.
    const switches = layout.controls.filter((control) => control.role === 'switch');
    const small = layout.controls.filter((control) => control.role !== 'switch' && control.height < 44);
    const outside = layout.controls.filter((control) => control.left < 0 || control.right > layout.width);
    await phone.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await openSettings(phone);
    await sleep(800);
    await phone.evaluate('window.__frames = 0');
    await sleep(1000);
    const framesIdle = await phone.evaluate('window.__frames');
    await typeInto(phone, '[data-backup-make] input[type=password]', 'short');
    await sleep(500);
    await phone.evaluate('window.__frames = 0');
    await sleep(1000);
    const framesTyped = await phone.evaluate('window.__frames');
    await phone.close();
    measured.touchSwitches = switches.map((control) => `${control.label}:${control.height}`);
    record("390×844 touch: Backup's controls (the shared Switch apart) are at least 44px and there is no horizontal page scroll; reduced motion leaves no animation frames",
      layout.coarse && layout.controls.length - switches.length >= 6 && switches.length === 3 && small.length === 0 && outside.length === 0 && layout.sideways <= 0 && framesIdle === 0 && framesTyped === 0,
      `coarse ${layout.coarse}; controls ${js(layout.controls.map((control) => `${control.label}:${control.height}`))}; under 44px ${js(small)}; outside ${js(outside)}; shared switches ${js(measured.touchSwitches)}; sideways ${layout.sideways}; frames idle ${framesIdle}, after typing ${framesTyped}`);
  });

  // 13. A 100 MB Blob through dump and restore, with the tab's peak memory (F28).
  if (process.env.BRUHS_VERIFY_SKIP_LARGE) {
    record('A 100 MB Blob makes a backup and restores byte for byte (peak renderer memory recorded)', false, 'skipped by BRUHS_VERIFY_SKIP_LARGE');
  } else {
    await check('A 100 MB Blob makes a backup and restores byte for byte (peak renderer memory recorded)', async () => {
      const largeKey = `bruhs.qa.large.${tag}`;
      await pageA.goto(`${A.base}/#/`);
      await pageA.appReady();
      await pageA.evaluate(putRecords({ metadata: [{ key: largeKey, revision: 1, value: { large: { $large: 100 } } }] }));
      const largeSha = await pageA.evaluate('window.__largeSha');
      await openSettings(pageA);
      const before = new Set(fs.readdirSync(downloads));
      await typeInto(pageA, '[data-backup-make] input[type=password]', password);
      await pageA.press('Tab');
      await pageA.type(password);
      await sleep(200);
      const madeAt = Date.now();
      const dump = await peakRenderer(async () => {
        await pageA.press('Enter');
        if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-made]') || !!document.querySelector('[data-backup-make-error]')?.innerText.trim()`, 600000))) throw new Error('the large backup never finished');
      });
      dump.seconds = Math.round((Date.now() - madeAt) / 100) / 10;
      const error = await text(pageA, '[data-backup-make-error]');
      const file = await until('the large download', () => {
        const fresh = fs.readdirSync(downloads).filter((name) => !before.has(name));
        return fresh.length === 1 && fresh[0].endsWith('.bruhsbak') ? fresh[0] : null;
      }, 180000).catch(() => null);
      if (!file) throw new Error(`no large download: ${error}`);
      const large = path.join(downloads, file);
      measured.largeFileMiB = Math.round(fs.statSync(large).size / 1048576);
      await openBackup(pageA, large, password);
      if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-preview]')`, 300000))) throw new Error(`no large preview: ${await text(pageA, '[data-backup-restore-error]')}`);
      const outcome = await replaceAndRestart(A, pageA, () => clickOn(pageA, '[data-backup-replace]'), 'a-large');
      if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-browser]')`, 20000))) throw new Error('no browser-data card on A');
      const restoreAt = Date.now();
      await pageA.evaluate(`window.__beforeRestore = 1`);
      const restore = await peakRenderer(async () => {
        await clickOn(pageA, '[data-backup-restore-browser]');
        await pageA.waitFor(`!!document.querySelector('[data-backup-browser-done]') || window.__beforeRestore !== 1`, 600000);
      });
      restore.seconds = Math.round((Date.now() - restoreAt) / 100) / 10;
      const restoreError = await text(pageA, '[data-backup-browser]');
      measured.largeRestoreReload = await peakRenderer(async () => {
        await pageA.waitFor(`window.__beforeRestore === undefined && document.readyState === 'complete'`, 120000);
        await pageA.appReady();
        await sleep(1500);
      });
      const back = await pageA.evaluate(describeRecord(largeKey));
      measured.largeDump = dump;
      measured.largeRestore = restore;
      record('A 100 MB Blob makes a backup and restores byte for byte (peak renderer memory recorded)',
        !error && outcome.back && back?.large?.sha === largeSha && back?.large?.size === 100 * 1048576,
        `file ${measured.largeFileMiB} MiB; dump ${js(dump)}; restore ${js(restore)}, its reload ${js(measured.largeRestoreReload)}; card said ${js(restoreError?.slice(0, 200))}; same bytes ${back?.large?.sha === largeSha}; exit ${js(outcome.timing)}`);
    });
  }

  // 15. N-I1: a HOME given with a trailing slash.
  let slashFile = null;
  await check('A HOME with a trailing slash (home-a/) makes a backup, which opens on the same server with no path rewrite (N-I1)', async () => {
    await stopChild(A.proc);
    A.homeEnv = `${A.home}/`;
    await startServer(A);
    await openSettings(pageA);
    // Check 13's 100 MB record goes first, so this backup stays small.
    await pageA.evaluate(`new Promise((resolve, reject) => {
      const open = indexedDB.open('bruhs');
      open.onsuccess = () => {
        const db = open.result;
        const tx = db.transaction('metadata', 'readwrite');
        tx.objectStore('metadata').delete(IDBKeyRange.bound('bruhs.qa.large.', 'bruhs.qa.large.\uffff'));
        tx.oncomplete = () => { db.close(); resolve(true); };
        tx.onerror = () => reject(new Error(String(tx.error)));
      };
      open.onerror = () => reject(new Error(String(open.error)));
    })`);
    const before = new Set(fs.readdirSync(downloads));
    await typeInto(pageA, '[data-backup-make] input[type=password]', password);
    await pageA.press('Tab');
    await pageA.type(password);
    await sleep(200);
    await pageA.press('Enter');
    if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-made]') || !!document.querySelector('[data-backup-make-error]')?.innerText.trim()`, 300000))) throw new Error('the backup never finished');
    const made = await text(pageA, '[data-backup-make-status]');
    const error = (await text(pageA, '[data-backup-make-error]'))?.trim();
    const file = await until('the download', () => fs.readdirSync(downloads).find((name) => !before.has(name) && name.endsWith('.bruhsbak')) ?? null, 120000).catch(() => null);
    if (file) slashFile = path.join(downloads, file);
    let lines = [];
    if (slashFile) {
      await openSettings(pageA);
      await openBackup(pageA, slashFile, password);
      if (await pageA.waitFor(`!!document.querySelector('[data-backup-preview]')`, 300000)) {
        lines = await pageA.evaluate(`[...document.querySelectorAll('[data-backup-preview-line]')].map((li) => li.innerText.trim())`);
        await clickOn(pageA, '[data-backup-preview] [data-backup-cancel]');
        await pageA.waitFor(`!document.querySelector('[data-backup-preview]')`, 15000);
      }
    }
    record('A HOME with a trailing slash (home-a/) makes a backup, which opens on the same server with no path rewrite (N-I1)',
      !error && /^Backup ready: /.test(made ?? '') && !!slashFile && lines.length > 0 && !lines.some((line) => line.startsWith('Paths under')),
      `HOME ${js(path.basename(A.homeEnv) + '/')}; result ${js(made)}; error ${js(error)}; file ${!!slashFile}; preview ${js(lines.filter((line) => !line.startsWith('Made')))}`);
  });

  // 16. The skipped-stores line, across the tab's reload.
  let pageS = null;
  await check("A browser restore into a database without one of the dump's stores names it, and the line shows after the tab's reload (WEB_STORES_SKIPPED)", async () => {
    if (!slashFile) throw new Error('no backup from check 15');
    await openSettings(pageA);
    await openBackup(pageA, slashFile, password);
    if (!(await pageA.waitFor(`!!document.querySelector('[data-backup-preview]')`, 300000))) throw new Error('no preview');
    const outcome = await replaceAndRestart(A, pageA, () => clickOn(pageA, '[data-backup-replace]'), 'a-skipped');
    // A fresh browser whose database has every store but whiteboard_assets,
    // at this build's version, made on a same-origin page that isn't the app.
    const contextS = await newContext(chrome.browser);
    pageS = await open(A, contextS);
    const css = (await (await fetch(`${A.base}/`)).text()).match(/href="\/?([^"]+\.css)"/)?.[1];
    await pageS.goto(`${A.base}/${css}`);
    await pageS.waitFor(`document.readyState === 'complete'`, 10000);
    const version = await pageS.evaluate(`new Promise((resolve, reject) => {
      const stores = ${js([['whiteboards', 'id'], ['whiteboard_index', 'id'], ['metadata', 'key'], ['threads', 'id'], ['messages', 'thread_id'], ['work_items', 'id'], ['projects', 'id'], ['events', 'id'], ['calendars', 'id'], ['meetings', 'id'], ['meeting_notes', 'id'], ['meeting_generations', 'id'], ['transcript_segments', 'id'], ['people', 'id'], ['captures', 'id'], ['knowledge_documents', 'id'], ['source_clips', 'id'], ['knowledge_attachments', 'id'], ['inbox_conversations', 'id'], ['relationships', 'person_id']])};
      const open = indexedDB.open('bruhs', 13);
      open.onupgradeneeded = () => {
        for (const [name, key] of stores) {
          const store = open.result.createObjectStore(name, { keyPath: key });
          if (name === 'transcript_segments') store.createIndex('by_meeting', 'meeting_id');
        }
      };
      open.onsuccess = () => { const db = open.result; const v = db.version; const names = [...db.objectStoreNames]; db.close(); resolve({ v, names: names.length }); };
      open.onerror = () => reject(new Error(String(open.error)));
    })`);
    await openSettings(pageS);
    if (!(await pageS.waitFor(`!!document.querySelector('[data-backup-browser]')`, 20000))) throw new Error('no browser-data card in the fresh browser');
    const field = await pageS.evaluate(`!!document.querySelector('[data-backup-browser] input[type=password]')`);
    await typeInto(pageS, '[data-backup-browser] input[type=password]', password);
    await pageS.evaluate(`window.__beforeRestore = 1`);
    await clickOn(pageS, '[data-backup-restore-browser]');
    const reloaded = await pageS.waitFor(`window.__beforeRestore === undefined && document.readyState === 'complete'`, 120000);
    await pageS.waitFor(settled, 60000);
    const shown = await pageS.waitFor(`document.getElementById('backup')?.innerText.includes(${js(COPY.skipped)})`, 10000);
    await pageS.evaluate(`document.getElementById('backup').scrollIntoView({ block: 'start' })`);
    await shoot(pageS, 'a-skipped-stores-1280x800');
    // Shown once: the next visit doesn't repeat it.
    await openSettings(pageS);
    await sleep(1500);
    const again = await pageS.evaluate(`document.getElementById('backup')?.innerText.includes(${js(COPY.skipped)})`);
    record("A browser restore into a database without one of the dump's stores names it, and the line shows after the tab's reload (WEB_STORES_SKIPPED)",
      outcome.back && version.v === 13 && version.names === 20 && field && reloaded && shown && !again,
      `restore ${js(outcome.restoredText)}; fresh database v${version.v} with ${version.names} stores; card asked for the password ${field}; reloaded ${reloaded}; line shown ${shown}; shown again on the next visit ${again}`);
  });

  // 17. D-I1: leaving Settings while the browser restore runs.
  await check('Leaving Settings mid browser restore leaves no panic and Settings opens again (D-I1)', async () => {
    if (!pageS) throw new Error('no page from check 16');
    // The set check 16's browser restore filled holds this browser's
    // previous data; putting it back waits on a browser restore again.
    const kept = (await overview(A)).pre_restores.filter((set) => set.has_web_part).sort((x, y) => y.made_at_ms - x.made_at_ms)[0];
    if (!kept) throw new Error('no kept set with browser data');
    const apply = await request(A, 'POST', '/api/backup/restore/apply', { source: { type: 'pre_restore', stamp: kept.stamp }, replace: true });
    if (apply.status !== 200) throw new Error(`put back: ${apply.status} ${apply.text}`);
    const exit = await Promise.race([A.exited, sleep(90000).then(() => null)]);
    if (!exit) throw new Error('A did not exit');
    await startServer(A);
    await openSettings(pageS);
    if (!(await pageS.waitFor(`!!document.querySelector('[data-backup-browser]')`, 20000))) throw new Error('no browser-data card after the put back');
    const panicsBefore = panics.length;
    const errorsBefore = pageS.runtimeErrors.length;
    await typeInto(pageS, '[data-backup-browser] input[type=password]', password);
    await clickOn(pageS, '[data-backup-restore-browser]');
    await pageS.evaluate(`location.hash = '#/'`);
    const leftAt = await pageS.evaluate(`({ hash: location.hash, done: !!document.querySelector('[data-backup-browser-done]') })`);
    await sleep(8000);
    await pageS.appReady(60000);
    const waiting = (await overview(A)).web_part_waiting;
    await openSettings(pageS);
    const again = await pageS.evaluate(`!!document.getElementById('backup')`);
    const newErrors = pageS.runtimeErrors.slice(errorsBefore).filter(Boolean).map((error) => String(error).slice(0, 160));
    record('Leaving Settings mid browser restore leaves no panic and Settings opens again (D-I1)',
      panics.length === panicsBefore && again,
      `left at ${js(leftAt)}; new panics ${panics.length - panicsBefore}; page errors since ${js(newErrors)}; browser part still waiting ${waiting}; Settings again ${again}`);
  });

  const pageErrors = pages.flatMap((page) => page.runtimeErrors.filter(Boolean).map((error) => String(error).slice(0, 200)));
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
  console.log(exitCode === 0 ? 'All backup checks passed.' : `${failed} backup check(s) failed.`);
}
process.exit(exitCode);
